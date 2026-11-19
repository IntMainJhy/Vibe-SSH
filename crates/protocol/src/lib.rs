//! Shared, transport-independent contract. These types do not grant remote authorization.

use serde::{Deserialize, Serialize};

/// Version 2 introduced task spaces and replaced the M0 per-channel leases.
pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_FRAME: usize = 2 * 1024 * 1024;
pub const MAX_COMMAND: usize = 1_048_576;
pub const MAX_INPUT: usize = 32_768;
pub const MAX_READ: usize = 1_048_576;
pub const DEFAULT_READ: usize = 8_192;
pub const MAX_WAIT_MS: u64 = 120_000;
pub const DEFAULT_FILE_ENTRIES: usize = 100;
pub const MAX_FILE_ENTRIES: usize = 1000;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const READ_CAPABILITIES: [&str; 3] = ["files_list", "files_read", "resources"];
pub const MAX_DEPLOYMENT_STEPS: usize = 12;
pub const MAX_LOG_READ: usize = 65536;

#[derive(Debug, Serialize, Deserialize)]
pub struct ResourceSample {
    pub cpu_percent: Option<f64>,
    pub cores: u32,
    pub uptime_s: u64,
    pub load: [f64; 3],
    pub memory_total: u64,
    pub memory_available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub disk_available: u64,
    #[serde(default)]
    pub mounts: Vec<MountUsage>,
    #[serde(default)]
    pub mounts_truncated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MountUsage {
    pub path: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub digest: Option<String>,
    pub excluded: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: Vec<ManifestEntry>,
    pub excludes: Vec<String>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffEntry {
    pub path: String,
    pub local_size: Option<u64>,
    pub remote_size: Option<u64>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffPreview {
    pub remote: String,
    pub entries: Vec<DiffEntry>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Agent,
    Human,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Succeeded,
    Failed,
    /// The remote outcome has no evidence. It is never reported as success.
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferKind {
    Upload,
    Download,
    UploadFolder,
    DownloadFolder,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileTransferRecord {
    pub kind: FileTransferKind,
    pub remote: String,
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentKind {
    Run,
    Put,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentStepSpec {
    pub name: String,
    pub kind: DeploymentKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeploymentAction {
    Run {
        command: String,
        cwd: Option<String>,
    },
    Put {
        remote_dir: String,
    },
}

impl DeploymentAction {
    pub fn kind(&self) -> DeploymentKind {
        match self {
            Self::Run { .. } => DeploymentKind::Run,
            Self::Put { .. } => DeploymentKind::Put,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
    Skipped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentStepRecord {
    #[serde(flatten)]
    pub plan: DeploymentStepSpec,
    pub status: DeploymentStepStatus,
    pub job_record: Option<String>,
    pub exit_code: Option<u32>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogArchive {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub issue: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogPage {
    pub text: String,
    pub offset: u64,
    pub next_offset: u64,
    pub archived_bytes: u64,
    pub observed_bytes: u64,
    pub complete: bool,
    pub invalid_utf8: bool,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub record: String,
    pub host: String,
    pub host_name: String,
    pub space_name: String,
    pub actor: String,
    pub origin: Owner,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub command_truncated: bool,
    pub pty: bool,
    pub started_unix_ms: u64,
    pub elapsed_ms: u64,
    pub status: JobStatus,
    pub reason: Option<String>,
    pub exit_code: Option<u32>,
    pub exit_signal: Option<String>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<FileTransferRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deployment: Vec<DeploymentStepRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<LogArchive>,
}
#[derive(Debug, Serialize)]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub more: bool,
}
#[derive(Debug, Serialize)]
pub struct HistoryDetail {
    #[serde(flatten)]
    pub record: HistoryRecord,
    pub stdout: Stream,
    pub stderr: Stream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ExitStatus,
    ExitSignal,
    MissingExitStatus,
    ConnectionLost,
    StartRejected,
    /// The upload stream was cut locally; the remote directory holds a partial copy.
    UploadIncomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    /// Desktop-only optimistic file version check failed.
    FileChanged,
    VersionMismatch,
    NotFound,
    /// The id, lease or cursor belongs to a service instance that no longer exists.
    InstanceMismatch,
    HostNotAuthorized,
    HostKeyMismatch,
    AuthenticationFailed,
    ConnectionFailed,
    /// The user holds the space. Stop writing to it.
    ControlRevoked,
    /// Control came back under a new lease. Read the space before writing again.
    StaleLease,
    /// Bytes may have been partially delivered. Do not blindly retry.
    DeliveryUnknown,
    ChannelRejected,
    ResourceLimit,
    ServiceUnavailable,
    Internal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Stream {
    pub text: String,
    pub total_bytes: u64,
    /// Bytes before the returned text were skipped or already rotated out.
    pub truncated: bool,
    pub invalid_utf8: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Control {
    pub owner: Owner,
    /// Pass this to every writing call. It changes on takeover, hand-off and release,
    /// and is absent while the user holds the space.
    pub lease: Option<String>,
    /// Why the agent handed the space to the user, if it did.
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    pub job: String,
    pub record: String,
    pub record_saved: bool,
    pub record_error: Option<String>,
    pub space: String,
    pub command: Option<String>,
    pub status: JobStatus,
    pub reason: Option<Reason>,
    pub exit_code: Option<u32>,
    pub exit_signal: Option<String>,
    pub interrupt_requested: bool,
    pub stdout: Stream,
    pub stderr: Stream,
    /// "merged" for PTY jobs, where stderr arrives inside stdout.
    pub stream_mode: &'static str,
    /// Pass to `job wait` to continue after the returned output.
    pub cursor: String,
    /// More output already exists beyond `cursor`.
    pub more: bool,
    pub control: Control,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobSummary {
    pub job: String,
    pub command: Option<String>,
    pub pty: bool,
    pub status: JobStatus,
    pub exit_code: Option<u32>,
    pub started_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SpaceView {
    pub space: String,
    pub name: String,
    pub host: String,
    pub host_name: String,
    /// Display label only. It is never used for authorization.
    pub actor: String,
    pub origin: Owner,
    pub connected: bool,
    pub control: Control,
    pub jobs: Vec<JobSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HostView {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub agent_access: bool,
}

/// One request per connection on the agent socket. Human operations are not expressible here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub call: Call,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Call {
    Status,
    Hosts,
    FilesList {
        space: String,
        remote: String,
        limit: usize,
    },
    FilesRead {
        space: String,
        remote: String,
        max_bytes: usize,
    },
    Resources {
        space: String,
    },
    HistoryList {
        offset: usize,
        limit: usize,
    },
    HistoryGet {
        record: String,
        tail_bytes: usize,
    },
    HistoryLog {
        record: String,
        stream: LogStream,
        offset: u64,
        limit: usize,
    },
    Diff {
        space: String,
        remote: String,
        manifest: Manifest,
    },
    SpaceOpen {
        host: String,
        name: String,
        /// Display label of the calling agent. Never used for authorization.
        actor: String,
    },
    SpaceList,
    SpaceGet {
        space: String,
    },
    SpaceClose {
        lease: String,
    },
    SpaceHandoff {
        lease: String,
        reason: String,
    },
    DeploymentStart {
        lease: String,
        steps: Vec<DeploymentStepSpec>,
    },
    DeploymentStep {
        lease: String,
        deployment: String,
        index: usize,
        action: DeploymentAction,
        wait_ms: u64,
    },
    DeploymentStop {
        lease: String,
        deployment: String,
        reason: String,
    },
    Run {
        lease: String,
        /// `None` starts a login shell and requires `pty`.
        command: Option<String>,
        cwd: Option<String>,
        pty: bool,
        wait_ms: u64,
        tail_bytes: usize,
    },
    JobGet {
        job: String,
        tail_bytes: usize,
    },
    JobWait {
        cursor: String,
        max_bytes: usize,
        timeout_ms: u64,
    },
    /// Followed on the same connection by the archive as length-prefixed frames.
    /// The service never opens local files on a caller's behalf.
    Put {
        lease: String,
        remote_dir: String,
        wait_ms: u64,
        tail_bytes: usize,
    },
    JobInput {
        lease: String,
        job: String,
        data: String,
        /// Close the job's stdin after writing.
        end: bool,
    },
    JobInterrupt {
        lease: String,
        job: String,
    },
}
