//! Shared, transport-independent types. These do not grant remote authorization.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_OUTPUT_LIMIT: usize = 1_048_576;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub pty: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Retained tail budget per output stream, not a remote output limit.
    #[serde(default = "default_output_limit")]
    pub output_limit: usize,
}

fn default_timeout() -> u64 {
    30_000
}

fn default_output_limit() -> usize {
    8_192
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionReason {
    ExitStatus,
    ExitSignal,
    MissingExitStatus,
    ObservationTimeout,
}

#[derive(Clone, Debug, Serialize)]
pub struct CapturedOutput {
    pub text: String,
    pub total_bytes: u64,
    pub retained_bytes: usize,
    pub truncated: bool,
    pub invalid_utf8: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionResult {
    pub job_id: Uuid,
    pub status: JobStatus,
    pub completion_reason: CompletionReason,
    pub exit_code: Option<u32>,
    pub exit_signal: Option<String>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub output_merged: bool,
    pub elapsed_ms: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputOwner {
    Agent,
    Human,
}

/// In-process concurrency capability, not an IPC authentication credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLease {
    id: Uuid,
    epoch: u64,
    owner: InputOwner,
}

impl InputLease {
    pub fn issue(epoch: u64, owner: InputOwner) -> Self {
        Self {
            id: Uuid::new_v4(),
            epoch,
            owner,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn owner(&self) -> InputOwner {
        self.owner
    }
}
