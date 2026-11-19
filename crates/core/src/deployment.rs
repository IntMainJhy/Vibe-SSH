use super::*;
use vibe_ssh_protocol::{
    DeploymentAction, DeploymentStepRecord, DeploymentStepSpec, DeploymentStepStatus as Step,
    MAX_DEPLOYMENT_STEPS,
};

const STEP_TAIL: usize = 2048;

pub(super) struct Deployment {
    lease: String,
    record: HistoryRecord,
    started: Instant,
}

impl Deployment {
    async fn save(&mut self, history: &Arc<History>) -> Result<()> {
        self.record.elapsed_ms = self.started.elapsed().as_millis() as u64;
        history.save(self.record.clone(), vec![], vec![]).await
    }

    fn end(&mut self, status: JobStatus, reason: String) {
        self.record.status = status;
        self.record.reason = Some(reason);
        for step in &mut self.record.deployment {
            if step.status == Step::Pending {
                step.status = Step::Skipped;
            }
        }
    }
}

impl Service {
    pub(crate) async fn deployment_start(
        &self,
        lease: &str,
        steps: Vec<DeploymentStepSpec>,
    ) -> Result<String> {
        if steps.is_empty()
            || steps.len() > MAX_DEPLOYMENT_STEPS
            || steps.iter().any(|step| {
                step.name.trim().is_empty()
                    || step.name.chars().count() > 80
                    || step.name.contains('\0')
            })
        {
            return err(
                ErrorCode::InvalidRequest,
                "deployment needs 1..=12 named steps",
            );
        }
        let by = By::Lease(lease);
        let space = self.space_of(&by)?;
        check(&*space.control.lock().await, &by)?;
        let mut current = space.deployment.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "a deployment step is still in progress",
            )
        })?;
        if let Some(mut previous) = current.take() {
            previous.end(JobStatus::Unknown, "superseded".into());
            previous.save(&self.history).await?;
        }
        let id = format!("h{}", token(32));
        let record = HistoryRecord {
            record: id.clone(),
            host: space.host.id.clone(),
            host_name: short(&space.host.name, 256).0,
            space_name: short(&space.name, 256).0,
            actor: space.actor.clone(),
            origin: Owner::Agent,
            command: None,
            cwd: None,
            command_truncated: false,
            pty: false,
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            elapsed_ms: 0,
            status: JobStatus::Running,
            reason: None,
            exit_code: None,
            exit_signal: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            transfer: None,
            archive: None,
            deployment: steps
                .into_iter()
                .map(|plan| DeploymentStepRecord {
                    plan,
                    status: Step::Pending,
                    job_record: None,
                    exit_code: None,
                    reason: None,
                })
                .collect(),
        };
        let mut deployment = Deployment {
            lease: lease.into(),
            record,
            started: Instant::now(),
        };
        deployment.save(&self.history).await?;
        *current = Some(deployment);
        bump(&self.events);
        Ok(id)
    }

    pub(crate) async fn deployment_step<A: AsyncRead + Unpin>(
        &self,
        lease: &str,
        deployment: &str,
        index: usize,
        action: DeploymentAction,
        wait_ms: u64,
        archive: &mut A,
    ) -> Result<JobView> {
        let by = By::Lease(lease);
        let space = self.space_of(&by)?;
        check(&*space.control.lock().await, &by)?;
        let mut current = space.deployment.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "a deployment step is still in progress",
            )
        })?;
        let deployment = current
            .as_mut()
            .filter(|active| active.lease == lease && active.record.record == deployment)
            .ok_or_else(|| {
                Error::new(ErrorCode::NotFound, "no active deployment for that lease")
            })?;
        let steps = &mut deployment.record.deployment;
        if steps.iter().position(|step| step.status == Step::Pending) != Some(index)
            || steps[index].plan.kind != action.kind()
            || steps[..index]
                .iter()
                .any(|step| step.status != Step::Succeeded)
        {
            return err(
                ErrorCode::InvalidRequest,
                "deployment step does not match the next approved plan slot",
            );
        }
        steps[index].status = Step::Running;
        if let Err(error) = deployment.save(&self.history).await {
            deployment.record.deployment[index].status = Step::Pending;
            return Err(error);
        }

        let started = match action {
            DeploymentAction::Run { command, cwd } => self.run(by, Some(command), cwd, None).await,
            DeploymentAction::Put { remote_dir } => self.put(by, archive, &remote_dir).await,
        };
        let result = match started {
            Ok(id) => {
                if let Ok(job) = self.find_job(&id, false) {
                    deployment.record.deployment[index].job_record =
                        Some(job.record.record.clone());
                    // Save the link even if the terminal result or the socket reply never arrives.
                    let _ = deployment.save(&self.history).await;
                }
                let wait = if check(&*space.control.lock().await, &by).is_ok() {
                    wait_ms
                } else {
                    0
                };
                self.job_settled(&id, wait, STEP_TAIL, true).await
            }
            Err(error) => Err(error),
        };

        let has_control = check(&*space.control.lock().await, &by).is_ok();
        let step = &mut deployment.record.deployment[index];
        match &result {
            Ok(job) => {
                step.job_record = Some(job.record.clone());
                step.exit_code = job.exit_code;
                step.reason = job
                    .reason
                    .and_then(|reason| serde_json::to_value(reason).ok())
                    .and_then(|reason| reason.as_str().map(str::to_owned));
                step.status = match job.status {
                    JobStatus::Running => Step::Running,
                    JobStatus::Succeeded => Step::Succeeded,
                    JobStatus::Failed => Step::Failed,
                    JobStatus::Unknown => Step::Unknown,
                };
                if job.status == JobStatus::Succeeded
                    && index + 1 == deployment.record.deployment.len()
                {
                    deployment.end(JobStatus::Succeeded, "all_steps_succeeded".into());
                } else if !has_control || job.status != JobStatus::Succeeded {
                    let reason = if !has_control {
                        "control_changed"
                    } else if job.status == JobStatus::Running {
                        "wait_expired"
                    } else {
                        "step_ended"
                    };
                    let status = if job.status == JobStatus::Failed {
                        JobStatus::Failed
                    } else {
                        JobStatus::Unknown
                    };
                    deployment.end(status, reason.into());
                }
            }
            Err(error) => {
                let unknown = matches!(
                    error.code,
                    ErrorCode::ConnectionFailed | ErrorCode::DeliveryUnknown | ErrorCode::NotFound
                );
                step.status = if unknown { Step::Unknown } else { Step::Failed };
                step.reason = Some(short(&error.message, 512).0);
                deployment.end(
                    if unknown {
                        JobStatus::Unknown
                    } else {
                        JobStatus::Failed
                    },
                    "step_rejected".into(),
                );
            }
        }
        let ended = deployment.record.status != JobStatus::Running;
        let saved = deployment.save(&self.history).await;
        if ended {
            *current = None;
        }
        bump(&self.events);
        saved.map_err(|error| {
            Error::new(
                ErrorCode::DeliveryUnknown,
                format!("deployment receipt could not be persisted: {error}"),
            )
        })?;
        result
    }

    pub(crate) async fn deployment_stop(
        &self,
        lease: &str,
        id: &str,
        reason: &str,
    ) -> Result<bool> {
        let by = By::Lease(lease);
        let space = self.space_of(&by)?;
        check(&*space.control.lock().await, &by)?;
        let mut current = space.deployment.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "a deployment step is still in progress",
            )
        })?;
        let Some(mut deployment) =
            current.take_if(|active| active.lease == lease && active.record.record == id)
        else {
            return Ok(false);
        };
        deployment.end(JobStatus::Unknown, short(reason, 512).0);
        deployment.save(&self.history).await?;
        bump(&self.events);
        Ok(true)
    }

    pub(super) fn invalidate_deployment(&self, space: &Arc<Space>) {
        if space.origin != Owner::Agent {
            return;
        }
        let (space, history, events) = (space.clone(), self.history.clone(), self.events.clone());
        tokio::spawn(async move {
            let mut current = space.deployment.lock().await;
            let control = space.control.lock().await;
            let invalid = current
                .as_ref()
                .is_some_and(|active| check(&control, &By::Lease(&active.lease)).is_err());
            drop(control);
            if invalid && let Some(mut deployment) = current.take() {
                deployment.end(JobStatus::Unknown, "control_changed".into());
                let _ = deployment.save(&history).await;
                bump(&events);
            }
        });
    }
}
