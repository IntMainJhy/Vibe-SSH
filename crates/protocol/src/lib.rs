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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Agent,
    Human,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Succeeded,
    Failed,
    /// The remote outcome has no evidence. It is never reported as success.
    Unknown,
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
