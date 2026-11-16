//! M0 SSH execution primitives. No daemon, persistent vault or UI is implied.

mod control;
mod output;
mod ssh;

pub use control::ControlledInput;
pub use ssh::{ConnectionConfig, RunningCommand, SshConnection};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    InvalidRequest(&'static str),
    #[error("SSH transport error: {0}")]
    Ssh(#[from] russh::Error),
    #[error("host key does not match the explicitly configured SHA256 fingerprint")]
    HostKeyMismatch,
    #[error("could not read the configured unencrypted OpenSSH identity")]
    IdentityLoad,
    #[error("SSH public-key authentication failed")]
    AuthenticationFailed,
    #[error("connection, authentication or channel setup deadline exceeded")]
    ConnectionTimeout,
    #[error("input control was revoked or the lease belongs to another channel")]
    ControlRevoked,
    #[error("input control epoch exhausted")]
    EpochExhausted,
    #[error("input submission timed out; delivery may be partial and must not be blindly retried")]
    InputDeliveryUnknown,
    #[error("remote channel rejected the requested operation")]
    ChannelRejected,
    #[error("SFTP operation failed: {0}")]
    Sftp(String),
}

pub type Result<T> = std::result::Result<T, Error>;
