use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Sig, client};
use serde_json::{Value, json};
use tokio::{
    io::AsyncReadExt,
    sync::{Notify, watch},
};
use uuid::Uuid;
use vibe_ssh_protocol::{
    Control, ErrorCode, HostView, JobStatus, JobSummary, JobView, MAX_COMMAND, MAX_INPUT, MAX_READ,
    MAX_WAIT_MS, Owner, PROTOCOL_VERSION, Reason, SpaceView,
};

use crate::{
    Error, Result,
    output::OutputLog,
    ssh::{Connection, Host, quote_posix},
};

/// Bound on one transport submission made while holding a space's control lock.
const SUBMIT: Duration = Duration::from_secs(3);
const LOG_LIMIT: usize = 1_048_576;
const MAX_SPACES: usize = 32;
/// OpenSSH allows 10 sessions per connection by default.
const MAX_RUNNING: usize = 8;
// ponytail: finished jobs live in memory until evicted; persist summaries when history matters.
const MAX_JOBS: usize = 64;

/// Returns the password or key passphrase for a host, if the embedding process has one.
pub type Secrets = Box<dyn Fn(&Host) -> Option<String> + Send + Sync>;

/// Who is writing. `Human` is only constructible by the trusted embedding process;
/// the agent socket can express nothing but leases.
#[derive(Clone, Copy)]
pub enum By<'a> {
    Lease(&'a str),
    Human(&'a str),
}

struct ControlState {
    owner: Owner,
    lease: String,
    note: Option<String>,
}

struct Space {
    id: u64,
    name: String,
    host: Host,
    actor: String,
    origin: Owner,
    conn: Connection,
    /// The lease check and the transport submission share this one serialization boundary.
    control: tokio::sync::Mutex<ControlState>,
    changed: watch::Sender<u64>,
}

struct Progress {
    stdout: OutputLog,
    stderr: OutputLog,
    status: JobStatus,
    reason: Option<Reason>,
    exit_code: Option<u32>,
    exit_signal: Option<String>,
    interrupt_requested: bool,
    elapsed_ms: Option<u64>,
}

struct Job {
    id: u64,
    space: Arc<Space>,
    command: Option<String>,
    pty: bool,
    started: Instant,
    started_unix_ms: u64,
    writer: ChannelWriteHalf<client::Msg>,
    /// Ends the job from this side. russh reports nothing back for a close we initiate.
    abort: Notify,
    progress: Mutex<Progress>,
}

#[derive(Default)]
struct Registry {
    next: u64,
    spaces: BTreeMap<u64, Arc<Space>>,
    jobs: BTreeMap<u64, Arc<Job>>,
}

pub struct Service {
    home: PathBuf,
    /// Ids, leases and cursors carry this tag so they cannot outlive a restart.
    instance: String,
    started: Instant,
    secrets: Secrets,
    registry: Mutex<Registry>,
    events: watch::Sender<u64>,
}

fn err<T>(code: ErrorCode, message: impl Into<String>) -> Result<T> {
    Err(Error::new(code, message))
}

fn token(length: usize) -> String {
    Uuid::new_v4().simple().to_string()[..length].to_owned()
}

fn bump(sender: &watch::Sender<u64>) {
    sender.send_modify(|version| *version += 1);
}

fn check(control: &ControlState, by: &By<'_>) -> Result<()> {
    match by {
        By::Human(_) if control.owner == Owner::Human => Ok(()),
        By::Human(_) => err(
            ErrorCode::ControlRevoked,
            "the agent holds this space; take over before writing",
        ),
        By::Lease(lease) if control.owner == Owner::Agent && control.lease == *lease => Ok(()),
        By::Lease(_) if control.owner == Owner::Human => err(
            ErrorCode::ControlRevoked,
            "the user holds this space; stop writing to it",
        ),
        By::Lease(_) => err(
            ErrorCode::StaleLease,
            "control came back under a new lease; read the space and re-check remote state before writing",
        ),
    }
}

async fn submit<F>(operation: F) -> Result<()>
where
    F: Future<Output = std::result::Result<(), russh::Error>>,
{
    match tokio::time::timeout(SUBMIT, operation).await {
        Ok(result) => Ok(result?),
        Err(_) => err(
            ErrorCode::DeliveryUnknown,
            "submission timed out; it may have been partially delivered, do not blindly retry",
        ),
    }
}

impl Service {
    pub fn new(home: PathBuf, secrets: Secrets) -> Arc<Self> {
        Arc::new(Self {
            home,
            instance: token(4),
            started: Instant::now(),
            secrets,
            registry: Mutex::default(),
            events: watch::Sender::new(0),
        })
    }

    pub fn home(&self) -> &PathBuf {
        &self.home
    }

    /// Fires when spaces, jobs or control change. Output alone does not fire it.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.events.subscribe()
    }

    pub fn status(&self) -> Value {
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": PROTOCOL_VERSION,
            "instance": self.instance,
            "pid": std::process::id(),
            "uptime_s": self.started.elapsed().as_secs(),
            "spaces": self.registry.lock().unwrap().spaces.len(),
        })
    }

    // ---- hosts: written only by the user through the embedding process ----

    pub fn load_hosts(&self) -> Result<Vec<Host>> {
        match std::fs::read(self.home.join("hosts.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::new(ErrorCode::InvalidRequest, format!("hosts.json: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => err(ErrorCode::Internal, format!("hosts.json: {e}")),
        }
    }

    pub fn save_hosts(&self, hosts: &[Host]) -> Result<()> {
        let io = |e: std::io::Error| Error::new(ErrorCode::Internal, format!("hosts.json: {e}"));
        let temporary = self.home.join("hosts.json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(hosts).unwrap()).map_err(io)?;
        std::fs::rename(&temporary, self.home.join("hosts.json")).map_err(io)?;
        bump(&self.events);
        Ok(())
    }

    pub fn hosts(&self, agent: bool) -> Result<Vec<HostView>> {
        Ok(self
            .load_hosts()?
            .into_iter()
            .filter(|host| host.agent_access || !agent)
            .map(|host| HostView {
                id: host.id,
                name: host.name,
                host: host.host,
                port: host.port,
                username: host.username,
                agent_access: host.agent_access,
            })
            .collect())
    }

    // ---- spaces ----

    pub async fn open(
        &self,
        host: &str,
        name: &str,
        actor: &str,
        origin: Owner,
    ) -> Result<SpaceView> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return err(
                ErrorCode::InvalidRequest,
                "space name must be 1..=80 characters",
            );
        }
        let agent = origin == Owner::Agent;
        let hosts = self.load_hosts()?;
        let mut found = hosts
            .iter()
            .filter(|h| (h.agent_access || !agent) && (h.id == host || h.name == host));
        let host = match (found.next(), found.next()) {
            (Some(host), None) => host.clone(),
            (Some(_), Some(_)) => {
                return err(
                    ErrorCode::InvalidRequest,
                    "several hosts share that name; use the host id",
                );
            }
            _ if agent => {
                return err(
                    ErrorCode::HostNotAuthorized,
                    "host is unknown or the user has not enabled agent access for it",
                );
            }
            _ => return err(ErrorCode::NotFound, "no such host"),
        };
        if self.registry.lock().unwrap().spaces.len() >= MAX_SPACES {
            return err(
                ErrorCode::ResourceLimit,
                "too many open spaces; close finished ones",
            );
        }
        // Each space owns its connection: an agent's traffic never shares a transport
        // with the user's terminals or with another agent.
        let conn = Connection::connect(&host, (self.secrets)(&host).as_deref()).await?;
        let space = {
            let mut registry = self.registry.lock().unwrap();
            registry.next += 1;
            let id = registry.next;
            let space = Arc::new(Space {
                id,
                name: name.to_owned(),
                host,
                actor: label(actor),
                origin,
                conn,
                control: tokio::sync::Mutex::new(ControlState {
                    owner: origin,
                    lease: self.lease(id),
                    note: None,
                }),
                changed: watch::Sender::new(0),
            });
            registry.spaces.insert(id, space.clone());
            space
        };
        bump(&self.events);
        Ok(self.space_view(&space).await)
    }

    pub async fn spaces(&self, agent: bool) -> Vec<SpaceView> {
        let spaces: Vec<_> = self
            .registry
            .lock()
            .unwrap()
            .spaces
            .values()
            .cloned()
            .collect();
        let mut views = Vec::new();
        for space in spaces.iter().filter(|s| s.origin == Owner::Agent || !agent) {
            views.push(self.space_view(space).await);
        }
        views
    }

    pub async fn space(&self, space: &str, agent: bool) -> Result<SpaceView> {
        let space = self.find_space(space, agent)?;
        Ok(self.space_view(&space).await)
    }

    /// Agents need the lease; the user may always close a space on their own machine.
    pub async fn close(&self, by: By<'_>) -> Result<()> {
        let space = self.space_of(&by)?;
        {
            let control = space.control.lock().await;
            if matches!(by, By::Lease(_)) {
                check(&control, &by)?;
            }
            let mut registry = self.registry.lock().unwrap();
            registry.spaces.remove(&space.id);
            registry.jobs.retain(|_, job| job.space.id != space.id);
        }
        space.conn.disconnect().await;
        bump(&space.changed);
        bump(&self.events);
        Ok(())
    }

    /// The user takes the space. Once this returns, no agent write can reach its channels.
    pub async fn takeover(&self, space: &str) -> Result<SpaceView> {
        let space = self.find_space(space, false)?;
        self.transfer(&space, Owner::Human, None).await;
        Ok(self.space_view(&space).await)
    }

    /// The user returns an agent space. The agent must read the space to learn the new lease.
    pub async fn release(&self, space: &str) -> Result<SpaceView> {
        let space = self.find_space(space, false)?;
        if space.origin == Owner::Human {
            return err(
                ErrorCode::InvalidRequest,
                "your own spaces cannot be given to an agent",
            );
        }
        self.transfer(&space, Owner::Agent, None).await;
        Ok(self.space_view(&space).await)
    }

    /// The agent asks the user to act (a password, a confirmation) and gives up control.
    pub async fn handoff(&self, lease: &str, reason: &str) -> Result<SpaceView> {
        let by = By::Lease(lease);
        let space = self.space_of(&by)?;
        {
            let mut control = space.control.lock().await;
            check(&control, &by)?;
            *control = ControlState {
                owner: Owner::Human,
                lease: self.lease(space.id),
                note: Some(reason.chars().take(500).collect()),
            };
        }
        bump(&space.changed);
        bump(&self.events);
        Ok(self.space_view(&space).await)
    }

    async fn transfer(&self, space: &Space, owner: Owner, note: Option<String>) {
        *space.control.lock().await = ControlState {
            owner,
            lease: self.lease(space.id),
            note,
        };
        bump(&space.changed);
        bump(&self.events);
    }

    // ---- jobs ----

    /// Starts a command, or a login shell when `command` is `None`. Returns the job id.
    pub async fn run(
        &self,
        by: By<'_>,
        command: Option<String>,
        cwd: Option<String>,
        pty: Option<(u32, u32)>,
    ) -> Result<String> {
        let invalid = |message| err(ErrorCode::InvalidRequest, message);
        match &command {
            Some(c) if c.is_empty() || c.contains('\0') || c.len() > MAX_COMMAND => {
                return invalid("command must be nonempty, NUL-free and at most 1 MiB");
            }
            None if pty.is_none() => return invalid("a shell without a command requires a PTY"),
            _ => {}
        }
        if cwd
            .as_ref()
            .is_some_and(|p| !p.starts_with('/') || p.contains('\0'))
        {
            return invalid("cwd must be an absolute NUL-free POSIX path");
        }
        if pty.is_some_and(|(columns, rows)| {
            !(1..=1000).contains(&columns) || !(1..=1000).contains(&rows)
        }) {
            return invalid("terminal dimensions must be 1..=1000");
        }
        let space = self.space_of(&by)?;
        check(&*space.control.lock().await, &by)?;
        if space.conn.is_closed() {
            return err(
                ErrorCode::ConnectionFailed,
                "the space's connection is closed; open a new space and check remote state before repeating changes",
            );
        }
        if self.running(space.id) >= MAX_RUNNING {
            return err(
                ErrorCode::ResourceLimit,
                "too many running jobs in this space",
            );
        }
        // Opening a channel has no remote effect, so it stays outside the control lock
        // and a slow server cannot delay a takeover.
        let channel = space.conn.channel(pty).await?;
        let line = match (&command, &cwd) {
            (Some(command), Some(cwd)) => Some(format!("cd {} && {command}", quote_posix(cwd))),
            (command, _) => command.clone(),
        };
        let started = {
            let control = space.control.lock().await;
            match check(&control, &by) {
                Ok(()) => match line {
                    Some(line) => submit(channel.exec(true, line)).await,
                    None => submit(channel.request_shell(true)).await,
                },
                Err(error) => Err(error),
            }
        };
        if let Err(error) = started {
            let _ = channel.close().await;
            return Err(error);
        }
        let (reader, writer) = channel.split();
        let job = {
            let mut registry = self.registry.lock().unwrap();
            registry.next += 1;
            let job = Arc::new(Job {
                id: registry.next,
                space: space.clone(),
                command,
                pty: pty.is_some(),
                started: Instant::now(),
                started_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64),
                writer,
                abort: Notify::new(),
                progress: Mutex::new(Progress {
                    stdout: OutputLog::new(LOG_LIMIT),
                    stderr: OutputLog::new(LOG_LIMIT),
                    status: JobStatus::Running,
                    reason: None,
                    exit_code: None,
                    exit_signal: None,
                    interrupt_requested: false,
                    elapsed_ms: None,
                }),
            });
            registry.jobs.insert(job.id, job.clone());
            let finished: Vec<u64> = registry
                .jobs
                .values()
                .filter(|j| j.space.id == space.id && !j.running())
                .map(|j| j.id)
                .collect();
            let held = registry
                .jobs
                .values()
                .filter(|j| j.space.id == space.id)
                .count();
            for id in finished.iter().take(held.saturating_sub(MAX_JOBS)) {
                registry.jobs.remove(id);
            }
            job
        };
        tokio::spawn(pump(job.clone(), reader, self.events.clone()));
        bump(&self.events);
        Ok(self.job_id(job.id))
    }

    /// Writes to the job's stdin. `end` closes stdin afterwards.
    pub async fn input(&self, by: By<'_>, job: &str, data: &[u8], end: bool) -> Result<()> {
        if data.len() > MAX_INPUT {
            return err(
                ErrorCode::InvalidRequest,
                "input chunks must not exceed 32768 bytes",
            );
        }
        let job = self.gated_job(&by, job)?;
        let control = job.space.control.lock().await;
        check(&control, &by)?;
        if !data.is_empty() {
            submit(job.writer.data(data)).await?;
        }
        if end {
            submit(job.writer.eof()).await?;
        }
        Ok(())
    }

    /// Uploads a local file or directory into `remote_dir` as one tar stream, as a job.
    /// Existing remote files that are not in the upload are never deleted.
    pub async fn put(
        &self,
        by: By<'_>,
        local: &Path,
        remote_dir: &str,
        excludes: &[String],
    ) -> Result<String> {
        if !remote_dir.starts_with('/') || remote_dir.contains('\0') {
            return err(
                ErrorCode::InvalidRequest,
                "remote directory must be an absolute NUL-free POSIX path",
            );
        }
        let local = local
            .canonicalize()
            .map_err(|e| Error::new(ErrorCode::NotFound, format!("local path: {e}")))?;
        let (root, entry) = match (local.is_dir(), local.parent(), local.file_name()) {
            (true, ..) => (local.as_path(), ".".as_ref()),
            (false, Some(parent), Some(name)) => (parent, name),
            _ => {
                return err(
                    ErrorCode::InvalidRequest,
                    "local path must be a file or directory",
                );
            }
        };
        let mut tar = tokio::process::Command::new("/usr/bin/tar");
        // Keep macOS resource forks and extended attributes out of the archive.
        tar.env("COPYFILE_DISABLE", "1")
            .args(["--no-xattrs", "--no-mac-metadata", "-cf", "-", "-C"])
            .arg(root);
        for pattern in excludes {
            tar.arg("--exclude").arg(pattern);
        }
        let mut tar = tar
            .arg("--")
            .arg(entry)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::new(ErrorCode::Internal, format!("local tar: {e}")))?;
        let directory = quote_posix(remote_dir);
        // -o: never apply the local machine's file ownership on the server.
        let command = format!("mkdir -p {directory} && tar -xof - -C {directory}");
        let id = self.run(by, Some(command), None, None).await?;
        let job = self.find_job(&id, false)?;
        let (agent, who) = match by {
            By::Lease(lease) => (true, lease.to_owned()),
            By::Human(space) => (false, space.to_owned()),
        };
        tokio::spawn(async move {
            let by = if agent {
                By::Lease(&who)
            } else {
                By::Human(&who)
            };
            let mut archive = tar.stdout.take().expect("piped");
            let mut chunk = vec![0; 16 * 1024];
            let stopped = loop {
                match archive.read(&mut chunk).await {
                    Ok(0) => break None,
                    Ok(read) => {
                        // Every chunk passes the same gate as typed input: a takeover stops the upload.
                        let control = job.space.control.lock().await;
                        let sent = match check(&control, &by) {
                            Ok(()) => submit(job.writer.data(&chunk[..read])).await,
                            Err(error) => Err(error),
                        };
                        if let Err(error) = sent {
                            break Some(error.message);
                        }
                    }
                    Err(error) => break Some(error.to_string()),
                }
            };
            // Close the pipe first: a tar blocked on a full pipe would otherwise never exit.
            drop(archive);
            if stopped.is_some() {
                let _ = tar.start_kill();
            }
            let local = tar.wait_with_output().await;
            let stopped = stopped.or_else(|| match local {
                Ok(output) if output.status.success() => None,
                Ok(output) => Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
                Err(error) => Some(error.to_string()),
            });
            match stopped {
                None => {
                    let _ = submit(job.writer.eof()).await;
                }
                Some(why) => {
                    // The remote archive is incomplete. Say so in the job instead of guessing.
                    let note = format!("\n[vibe-ssh] upload stopped before completion: {why}\n");
                    job.progress.lock().unwrap().stderr.append(note.as_bytes());
                    // A cut on a record boundary can still unpack cleanly, so the remote
                    // exit status must not get to call this upload a success.
                    job.abort.notify_one();
                }
            }
        });
        Ok(id)
    }

    pub async fn resize(&self, by: By<'_>, job: &str, columns: u32, rows: u32) -> Result<()> {
        if !(1..=1000).contains(&columns) || !(1..=1000).contains(&rows) {
            return err(
                ErrorCode::InvalidRequest,
                "terminal dimensions must be 1..=1000",
            );
        }
        let job = self.gated_job(&by, job)?;
        let control = job.space.control.lock().await;
        check(&control, &by)?;
        submit(job.writer.window_change(columns, rows, 0, 0)).await
    }

    /// Requests an interrupt. The job stays `running` until the remote actually ends it.
    pub async fn interrupt(&self, by: By<'_>, job: &str) -> Result<()> {
        let job = self.gated_job(&by, job)?;
        let control = job.space.control.lock().await;
        check(&control, &by)?;
        job.progress.lock().unwrap().interrupt_requested = true;
        bump(&job.space.changed);
        if job.pty {
            submit(job.writer.data(&b"\x03"[..])).await
        } else {
            submit(job.writer.signal(Sig::INT)).await
        }
    }

    /// The job with its last `tail` bytes of output.
    pub async fn job(&self, job: &str, tail: usize, agent: bool) -> Result<JobView> {
        let job = self.find_job(job, agent)?;
        Ok(self.job_view(&job, None, tail).await)
    }

    /// Returns once the job has output past `cursor`, finishes, changes hands, or the wait expires.
    /// An expired wait is not a failure: the job is simply still running.
    pub async fn job_wait(
        &self,
        cursor: &str,
        max: usize,
        timeout_ms: u64,
        agent: bool,
    ) -> Result<JobView> {
        let bad = || Error::new(ErrorCode::InvalidRequest, "malformed cursor");
        let mut parts = cursor.split(':');
        let job = self.find_job(parts.next().ok_or_else(bad)?, agent)?;
        let mut offset = || {
            parts
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .ok_or_else(bad)
        };
        let at = (offset()?, offset()?);
        settle(&job, timeout_ms, |p| {
            // A cursor past the end is answered at once so the caller sees it flagged.
            p.stdout.read(at.0, 16).1 != at.0 || p.stderr.read(at.1, 16).1 != at.1
        })
        .await;
        Ok(self.job_view(&job, Some(at), max).await)
    }

    /// Waits for the job to finish, then returns it with its last `tail` bytes.
    pub async fn job_settled(
        &self,
        job: &str,
        wait_ms: u64,
        tail: usize,
        agent: bool,
    ) -> Result<JobView> {
        let job = self.find_job(job, agent)?;
        if wait_ms > 0 {
            settle(&job, wait_ms, |_| false).await;
        }
        Ok(self.job_view(&job, None, tail).await)
    }

    /// Raw bytes for a terminal renderer: `(bytes, next offset, a gap precedes them)`.
    pub fn job_bytes(&self, job: &str, from: u64) -> Result<(Vec<u8>, u64, bool)> {
        let job = self.find_job(job, false)?;
        let progress = job.progress.lock().unwrap();
        Ok(progress.stdout.bytes(from, MAX_READ))
    }

    /// Fires on output, completion and control changes of the job's space.
    pub fn job_events(&self, job: &str) -> Result<watch::Receiver<u64>> {
        Ok(self.find_job(job, false)?.space.changed.subscribe())
    }

    // ---- internals ----

    fn space_id(&self, id: u64) -> String {
        format!("s{id}.{}", self.instance)
    }

    fn job_id(&self, id: u64) -> String {
        format!("j{id}.{}", self.instance)
    }

    fn lease(&self, space: u64) -> String {
        format!("{}.{}", self.space_id(space), token(12))
    }

    fn parse(&self, text: &str, prefix: char) -> Result<u64> {
        let invalid = || Error::new(ErrorCode::InvalidRequest, format!("malformed id {text:?}"));
        let (number, instance) = text
            .strip_prefix(prefix)
            .and_then(|rest| rest.split_once('.'))
            .ok_or_else(invalid)?;
        if instance != self.instance {
            return err(
                ErrorCode::InstanceMismatch,
                "this id belongs to a service instance that has ended; its spaces and jobs are gone",
            );
        }
        number.parse().map_err(|_| invalid())
    }

    fn find_space(&self, space: &str, agent: bool) -> Result<Arc<Space>> {
        let id = self.parse(space, 's')?;
        let found = self.registry.lock().unwrap().spaces.get(&id).cloned();
        // The user's own spaces are invisible to agents.
        found
            .filter(|space| space.origin == Owner::Agent || !agent)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::NotFound,
                    "no such space; it may have been closed",
                )
            })
    }

    fn space_of(&self, by: &By<'_>) -> Result<Arc<Space>> {
        match by {
            By::Human(space) => self.find_space(space, false),
            By::Lease(lease) => {
                self.find_space(lease.rsplit_once('.').map_or(lease, |p| p.0), true)
            }
        }
    }

    fn find_job(&self, job: &str, agent: bool) -> Result<Arc<Job>> {
        let id = self.parse(job, 'j')?;
        let found = self.registry.lock().unwrap().jobs.get(&id).cloned();
        found
            .filter(|job| job.space.origin == Owner::Agent || !agent)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::NotFound,
                    "no such job; it may have been evicted or its space closed",
                )
            })
    }

    fn gated_job(&self, by: &By<'_>, job: &str) -> Result<Arc<Job>> {
        let job = self.find_job(job, matches!(by, By::Lease(_)))?;
        if job.space.id != self.space_of(by)?.id {
            return err(ErrorCode::InvalidRequest, "the job is not in that space");
        }
        if !job.running() {
            return err(ErrorCode::InvalidRequest, "the job has already finished");
        }
        Ok(job)
    }

    fn running(&self, space: u64) -> usize {
        let registry = self.registry.lock().unwrap();
        registry
            .jobs
            .values()
            .filter(|j| j.space.id == space && j.running())
            .count()
    }

    async fn control_view(&self, space: &Space) -> Control {
        let control = space.control.lock().await;
        Control {
            owner: control.owner,
            // The user's lease is never shown: nothing an agent can read grants it input.
            lease: (control.owner == Owner::Agent).then(|| control.lease.clone()),
            note: control.note.clone(),
        }
    }

    async fn space_view(&self, space: &Space) -> SpaceView {
        let control = self.control_view(space).await;
        let registry = self.registry.lock().unwrap();
        SpaceView {
            space: self.space_id(space.id),
            name: space.name.clone(),
            host: space.host.id.clone(),
            host_name: space.host.name.clone(),
            actor: space.actor.clone(),
            origin: space.origin,
            connected: !space.conn.is_closed(),
            control,
            jobs: registry
                .jobs
                .values()
                .filter(|job| job.space.id == space.id)
                .map(|job| {
                    let progress = job.progress.lock().unwrap();
                    JobSummary {
                        job: self.job_id(job.id),
                        command: job.command.clone(),
                        pty: job.pty,
                        status: progress.status,
                        exit_code: progress.exit_code,
                        started_unix_ms: job.started_unix_ms,
                    }
                })
                .collect(),
        }
    }

    async fn job_view(&self, job: &Job, at: Option<(u64, u64)>, max: usize) -> JobView {
        let max = max.min(MAX_READ);
        let control = self.control_view(&job.space).await;
        let progress = job.progress.lock().unwrap();
        let ((stdout, out), (stderr, error)) = match at {
            Some((out, error)) => (
                progress.stdout.read(out, max),
                progress.stderr.read(error, max),
            ),
            None => (progress.stdout.tail(max), progress.stderr.tail(max)),
        };
        // Up to three trailing bytes can be a character still in flight.
        let more = progress.stdout.total() - out > 3 || progress.stderr.total() - error > 3;
        let id = self.job_id(job.id);
        JobView {
            cursor: format!("{id}:{out}:{error}"),
            job: id,
            space: self.space_id(job.space.id),
            command: job.command.clone(),
            status: progress.status,
            reason: progress.reason,
            exit_code: progress.exit_code,
            exit_signal: progress.exit_signal.clone(),
            interrupt_requested: progress.interrupt_requested,
            stdout,
            stderr,
            stream_mode: if job.pty { "merged" } else { "separate" },
            more,
            control,
            elapsed_ms: progress
                .elapsed_ms
                .unwrap_or_else(|| job.started.elapsed().as_millis() as u64),
        }
    }
}

impl Job {
    fn running(&self) -> bool {
        self.progress.lock().unwrap().status == JobStatus::Running
    }
}

/// Waits until the job finishes, `ready` holds, control changes hands, or the time runs out.
async fn settle(job: &Job, timeout_ms: u64, ready: impl Fn(&Progress) -> bool) {
    let mut changed = job.space.changed.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    let lease = job.space.control.lock().await.lease.clone();
    loop {
        let done = {
            let progress = job.progress.lock().unwrap();
            progress.status != JobStatus::Running || ready(&progress)
        };
        if done || job.space.control.lock().await.lease != lease {
            return;
        }
        if !matches!(
            tokio::time::timeout_at(deadline, changed.changed()).await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
}

/// Drains one channel into the job. Reading never needs control: output stays observable.
async fn pump(job: Arc<Job>, mut reader: ChannelReadHalf, events: watch::Sender<u64>) {
    let mut exit_code = None;
    let mut exit_signal = None;
    let outcome = loop {
        let message = tokio::select! {
            _ = job.abort.notified() => break (JobStatus::Failed, Reason::UploadIncomplete),
            message = reader.wait() => message,
        };
        let (to_stderr, data) = match message {
            Some(ChannelMsg::Data { data }) => (false, data),
            Some(ChannelMsg::ExtendedData { data, .. }) => (!job.pty, data),
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                exit_code = Some(exit_status);
                continue;
            }
            Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                exit_signal = Some(format!("{signal_name:?}"));
                continue;
            }
            // Only exec and shell requests ask for a reply.
            Some(ChannelMsg::Failure) => break (JobStatus::Failed, Reason::StartRejected),
            // Data may follow the exit status, so only closure ends the job.
            Some(ChannelMsg::Close) => {
                break match (&exit_signal, exit_code) {
                    (Some(_), _) => (JobStatus::Failed, Reason::ExitSignal),
                    (None, Some(0)) => (JobStatus::Succeeded, Reason::ExitStatus),
                    (None, Some(_)) => (JobStatus::Failed, Reason::ExitStatus),
                    // No exit status is no evidence. It is never turned into success.
                    (None, None) => (JobStatus::Unknown, Reason::MissingExitStatus),
                };
            }
            // The transport ended under the job. The remote process may still be running.
            None => break (JobStatus::Unknown, Reason::ConnectionLost),
            Some(_) => continue,
        };
        let mut progress = job.progress.lock().unwrap();
        if to_stderr {
            progress.stderr.append(&data);
        } else {
            progress.stdout.append(&data);
        }
        drop(progress);
        bump(&job.space.changed);
    };
    let _ = job.writer.close().await;
    {
        let mut progress = job.progress.lock().unwrap();
        (progress.status, progress.reason) = (outcome.0, Some(outcome.1));
        progress.exit_code = exit_code;
        progress.exit_signal = exit_signal;
        progress.elapsed_ms = Some(job.started.elapsed().as_millis() as u64);
        progress.stdout.closed = true;
        progress.stderr.closed = true;
    }
    bump(&job.space.changed);
    bump(&events);
}

/// Display label for the calling agent. Never used for authorization.
fn label(actor: &str) -> String {
    let label: String = actor
        .chars()
        .filter(|c| c.is_alphanumeric() || "-_. ".contains(*c))
        .take(32)
        .collect();
    if label.trim().is_empty() {
        "agent".into()
    } else {
        label
    }
}
