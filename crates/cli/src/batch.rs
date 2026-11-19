use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};
use vibe_ssh_core::{Archive, Error, ErrorCode, Result, request};
use vibe_ssh_protocol::{
    Call, DeploymentAction, DeploymentKind, DeploymentStepSpec, MAX_DEPLOYMENT_STEPS, MAX_WAIT_MS,
};

use crate::compact_job;

const MAX_PLAN: usize = 256 * 1024;

#[derive(Args)]
pub(crate) struct Batch {
    /// Existing agent space lease.
    #[arg(long, conflicts_with = "host", required_unless_present = "host")]
    lease: Option<String>,
    /// Open one new task space on this authorized host id or unique name.
    #[arg(long, requires = "name")]
    host: Option<String>,
    /// Goal shown in the app when opening a space.
    #[arg(long, requires = "host")]
    name: Option<String>,
    /// Display name for a new space.
    #[arg(long, env = "VIBE_SSH_ACTOR", default_value = "agent")]
    actor: String,
    /// Seconds to wait for each step. If it is still running, later steps remain unsubmitted.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=120))]
    wait: u64,
    /// Local JSON plan, or '-' for standard input.
    #[arg(default_value = "-")]
    plan: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Run {
        name: String,
        command: String,
        cwd: Option<String>,
    },
    Put {
        name: String,
        local: PathBuf,
        remote_dir: String,
        #[serde(default)]
        exclude: Vec<String>,
    },
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn read_plan(mut reader: impl Read) -> Result<Plan> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_PLAN as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PLAN {
        return Err(invalid("batch plan exceeds 256 KiB"));
    }
    let plan: Plan =
        serde_json::from_slice(&bytes).map_err(|e| invalid(format!("batch plan: {e}")))?;
    if !(1..=MAX_DEPLOYMENT_STEPS).contains(&plan.steps.len()) {
        return Err(invalid("batch requires 1..=12 steps"));
    }
    for (index, step) in plan.steps.iter().enumerate() {
        if let Err(error) = step.validate() {
            return Err(invalid(format!("batch step {}: {error}", index + 1)));
        }
    }
    Ok(plan)
}

impl Step {
    fn name(&self) -> &str {
        match self {
            Self::Run { name, .. } | Self::Put { name, .. } => name,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.name().trim().is_empty()
            || self.name().chars().count() > 80
            || self.name().contains('\0')
        {
            return Err(invalid("name must be 1..=80 characters"));
        }
        let absolute = |path: &str| path.starts_with('/') && !path.contains('\0');
        match self {
            Self::Run { command, cwd, .. } => {
                if command.trim().is_empty() || command.contains('\0') {
                    return Err(invalid("expected a nonempty remote command"));
                }
                if cwd.as_deref().is_some_and(|dir| !absolute(dir)) {
                    return Err(invalid("remote cwd must be absolute"));
                }
            }
            Self::Put {
                local,
                remote_dir,
                exclude,
                ..
            } => {
                if local.as_os_str().is_empty()
                    || !absolute(remote_dir)
                    || exclude.iter().any(|pattern| pattern.contains('\0'))
                {
                    return Err(invalid(
                        "expected a local path, an absolute remote directory and valid excludes",
                    ));
                }
                let meta = local.metadata().map_err(|e| {
                    Error::new(ErrorCode::NotFound, format!("local upload source: {e}"))
                })?;
                if !meta.is_file() && !meta.is_dir() {
                    return Err(invalid("local upload source must be a file or directory"));
                }
            }
        }
        Ok(())
    }

    fn spec(&self) -> DeploymentStepSpec {
        let kind = match self {
            Self::Run { .. } => DeploymentKind::Run,
            Self::Put { .. } => DeploymentKind::Put,
        };
        DeploymentStepSpec {
            name: self.name().into(),
            kind,
        }
    }

    async fn submit(
        self,
        home: &Path,
        lease: &str,
        deployment: &str,
        index: usize,
        wait_ms: u64,
    ) -> Result<Value> {
        let (action, archive) = match self {
            Self::Run { command, cwd, .. } => (DeploymentAction::Run { command, cwd }, None),
            Self::Put {
                local,
                remote_dir,
                exclude,
                ..
            } => (
                DeploymentAction::Put { remote_dir },
                Some(Archive::pack(&local, &exclude)?),
            ),
        };
        request(
            home,
            Call::DeploymentStep {
                lease: lease.into(),
                deployment: deployment.into(),
                index,
                action,
                wait_ms,
            },
            archive,
        )
        .await
    }
}

impl Batch {
    pub(crate) async fn run(self, home: &Path) -> Result<u8> {
        let plan = if self.plan.as_os_str() == "-" {
            read_plan(io::stdin().lock())?
        } else {
            let file = File::open(&self.plan)
                .map_err(|e| Error::new(ErrorCode::NotFound, format!("batch plan: {e}")))?;
            read_plan(file)?
        };
        let (lease, mut space) = match (self.host, self.name, self.lease) {
            (Some(host), Some(name), None) => {
                let opened = request(
                    home,
                    Call::SpaceOpen {
                        host,
                        name,
                        actor: self.actor,
                    },
                    None,
                )
                .await?;
                let lease = opened["control"]["lease"].as_str().ok_or_else(|| {
                    Error::new(
                        ErrorCode::Internal,
                        "new space did not return an agent lease",
                    )
                })?;
                (lease.to_owned(), opened["space"].clone())
            }
            (None, None, Some(lease)) => (lease, Value::Null),
            _ => return Err(invalid("choose a host and goal or an existing lease")),
        };
        let mut steps = Vec::with_capacity(plan.steps.len());
        let count = plan.steps.len();
        let opened = request(
            home,
            Call::DeploymentStart {
                lease: lease.clone(),
                steps: plan.steps.iter().map(Step::spec).collect(),
            },
            None,
        )
        .await;
        let deployment = match opened {
            Ok(value) => value["deployment"]
                .as_str()
                .ok_or_else(|| invalid("service did not return a deployment record"))?
                .to_owned(),
            Err(error) => {
                steps.push(json!({"name":plan.steps[0].name(),"error":{"code":error.code,"message":error.message}}));
                println!(
                    "{}",
                    json!({"space":space,"outcome":"stopped","unsubmitted_steps":count,"steps":steps})
                );
                return Ok(2);
            }
        };
        let wait_ms = (self.wait * 1000).min(MAX_WAIT_MS);
        let mut code = 0;
        for (index, step) in plan.steps.into_iter().enumerate() {
            let name = step.name().to_owned();
            match step.submit(home, &lease, &deployment, index, wait_ms).await {
                Ok(reply) => {
                    space = reply["space"].clone();
                    steps.push(json!({ "name": name, "result": compact_job(&reply) }));
                    if reply["status"] != "succeeded" || reply["control"]["owner"] != "agent" {
                        code = 1;
                        break;
                    }
                }
                Err(error) => {
                    let _ = request(
                        home,
                        Call::DeploymentStop {
                            lease: lease.clone(),
                            deployment: deployment.clone(),
                            reason: "agent_could_not_continue".into(),
                        },
                        None,
                    )
                    .await;
                    steps.push(json!({ "name": name, "error": { "code": error.code, "message": error.message } }));
                    code = 2;
                    break;
                }
            }
        }
        println!(
            "{}",
            json!({
                "space": space,
                "deployment": deployment,
                "outcome": if code == 0 { "completed" } else { "stopped" },
                "unsubmitted_steps": count - steps.len(),
                "steps": steps,
            })
        );
        Ok(code)
    }
}
