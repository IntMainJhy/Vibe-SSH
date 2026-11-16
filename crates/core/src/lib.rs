//! Task-space SSH kernel shared by the local agent socket and the desktop app.

mod output;
mod server;
mod service;
pub mod skill;
mod ssh;

pub use server::{default_home, request, serve, socket_path};
pub use service::{By, Secrets, Service};
pub use ssh::Host;
pub use vibe_ssh_protocol::ErrorCode;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<russh::Error> for Error {
    fn from(error: russh::Error) -> Self {
        Self::new(
            ErrorCode::ConnectionFailed,
            format!("SSH transport: {error}"),
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
