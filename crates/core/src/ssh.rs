use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{
    Channel, ChannelMsg, client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key},
};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use vibe_ssh_protocol::ErrorCode;

use crate::{Error, Result};

pub(crate) const SETUP: Duration = Duration::from_secs(15);

/// A server the user added by hand. Secrets are never stored here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    /// OpenSSH private key. Without it the connection uses password authentication.
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
    /// Fingerprint the user verified out of band. A mismatch stops before authentication.
    pub host_key_sha256: String,
    /// Agents can only see and open hosts the user enabled.
    #[serde(default)]
    pub agent_access: bool,
}

fn default_port() -> u16 {
    22
}

struct Pinned {
    fingerprint: String,
}

impl client::Handler for Pinned {
    type Error = Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        if key.public_key().fingerprint(HashAlg::Sha256).to_string() != self.fingerprint {
            return Err(Error::new(
                ErrorCode::HostKeyMismatch,
                "host key does not match the fingerprint the user verified",
            ));
        }
        Ok(true)
    }
}

struct KeyProbe(Arc<Mutex<Option<(String, String)>>>);

impl client::Handler for KeyProbe {
    type Error = Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        let key = key.public_key();
        *self.0.lock().unwrap() = Some((
            key.algorithm().to_string(),
            key.fingerprint(HashAlg::Sha256).to_string(),
        ));
        // Never proceed to authentication: this connection only exists to show the key.
        Ok(false)
    }
}

/// Fetches a server's host key as `(algorithm, fingerprint)` for the user to verify out of band.
/// Human-only: it is not reachable from the agent socket, and it sends no credential.
pub async fn host_key(host: &str, port: u16) -> Result<(String, String)> {
    let seen = Arc::new(Mutex::new(None));
    let probe = client::connect(Arc::default(), (host, port), KeyProbe(seen.clone()));
    let _ = timeout(SETUP, probe).await;
    let fingerprint = seen.lock().unwrap().take();
    fingerprint.ok_or_else(|| {
        Error::new(
            ErrorCode::ConnectionFailed,
            "could not reach the server or read its host key",
        )
    })
}

pub(crate) struct Connection {
    handle: client::Handle<Pinned>,
}

impl Connection {
    /// `secret` yields the password, or the passphrase of an encrypted identity file.
    /// It is only consulted when the credential cannot be used without it.
    pub async fn connect(
        host: &Host,
        secret: &(dyn Fn() -> Option<String> + Sync),
    ) -> Result<Self> {
        let invalid = |message| Error::new(ErrorCode::InvalidRequest, message);
        if host.host.is_empty() || host.username.is_empty() || host.port == 0 {
            return Err(invalid("host, username and a nonzero port are required"));
        }
        if !host.host_key_sha256.starts_with("SHA256:") {
            return Err(invalid(
                "a user-verified SHA256 host fingerprint is required",
            ));
        }
        let failed = |message| Error::new(ErrorCode::AuthenticationFailed, message);
        let key = match &host.identity_file {
            Some(path) => Some(
                load_secret_key(path, None)
                    .or_else(|_| load_secret_key(path, secret().as_deref()))
                    .map_err(|_| failed("could not read the configured OpenSSH identity"))?,
            ),
            None => None,
        };
        let config = client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            nodelay: true,
            ..Default::default()
        };
        let connect = async {
            let mut handle = client::connect(
                Arc::new(config),
                (host.host.as_str(), host.port),
                Pinned {
                    fingerprint: host.host_key_sha256.clone(),
                },
            )
            .await?;
            let result = match key {
                Some(key) => {
                    let hash = handle.best_supported_rsa_hash().await?.flatten();
                    handle
                        .authenticate_publickey(
                            &host.username,
                            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                        )
                        .await?
                }
                None => {
                    let password = secret().ok_or_else(|| failed("no credential is available"))?;
                    handle
                        .authenticate_password(&host.username, password)
                        .await?
                }
            };
            if !result.success() {
                return Err(failed("the server rejected the credential"));
            }
            Ok(Self { handle })
        };
        timeout(SETUP, connect).await.map_err(|_| {
            Error::new(
                ErrorCode::ConnectionFailed,
                "connection or authentication timed out",
            )
        })?
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Opens a session channel. Nothing runs on the remote until the caller sends exec or shell.
    pub async fn channel(&self, pty: Option<(u32, u32)>) -> Result<Channel<client::Msg>> {
        let mut channel = timeout(SETUP, self.handle.channel_open_session())
            .await
            .map_err(|_| Error::new(ErrorCode::ConnectionFailed, "channel open timed out"))??;
        if let Some((columns, rows)) = pty {
            channel
                .request_pty(true, "xterm-256color", columns, rows, 0, 0, &[])
                .await?;
            await_reply(&mut channel).await?;
        }
        Ok(channel)
    }

    pub async fn sftp(&self) -> Result<(russh_sftp::client::RawSftpSession, bool)> {
        let mut channel = self.channel(None).await?;
        channel.request_subsystem(true, "sftp").await?;
        await_reply(&mut channel).await?;
        let session = russh_sftp::client::RawSftpSession::new(channel.into_stream());
        session.set_timeout(3);
        let version = session
            .init()
            .await
            .map_err(|e| Error::new(ErrorCode::ChannelRejected, format!("SFTP: {e}")))?;
        let extensions = ["posix-rename@openssh.com", "hardlink@openssh.com"];
        let editable = extensions
            .iter()
            .all(|name| version.extensions.get(*name).is_some_and(|v| v == "1"));
        Ok((session, editable))
    }

    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "space closed", "en")
            .await;
    }
}

async fn await_reply(channel: &mut Channel<client::Msg>) -> Result<()> {
    let rejected = || {
        Error::new(
            ErrorCode::ChannelRejected,
            "the server rejected the request",
        )
    };
    timeout(SETUP, async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::WindowAdjusted { .. }) => {}
                _ => return Err(rejected()),
            }
        }
    })
    .await
    .map_err(|_| Error::new(ErrorCode::ConnectionFailed, "channel request timed out"))?
}

pub(crate) fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn quotes_actual_posix_directory_without_executing_its_contents() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("space ' quote $HOME `whoami`");
        std::fs::create_dir(&directory).unwrap();
        let command = format!(
            "cd {} && printf '%s' \"$PWD\"",
            quote_posix(directory.to_str().unwrap())
        );
        let result = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            directory.to_str().unwrap()
        );
    }
}
