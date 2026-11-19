use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{
    Channel, ChannelMsg, client,
    keys::{
        HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate,
        agent::{AgentIdentity, client::AgentClient},
        load_secret_key,
    },
};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use vibe_ssh_protocol::ErrorCode;

use crate::{Error, ProxySettings, Result};

pub(crate) const SETUP: Duration = Duration::from_secs(15);

#[path = "remote_forward.rs"]
mod remote_forward;
use remote_forward::RemoteRoutes;

/// A server the user added by hand. Secrets are never stored here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_agent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
    /// Fingerprint the user verified out of band. A mismatch stops before authentication.
    pub host_key_sha256: String,
    /// Only enabled hosts can grant agent write access.
    #[serde(default)]
    pub agent_access: bool,
}

fn default_port() -> u16 {
    22
}

struct Pinned {
    fingerprint: String,
    remote_routes: Arc<RemoteRoutes>,
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

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<()> {
        self.remote_routes.relay(
            connected_address,
            connected_port,
            originator_address,
            channel,
            reply,
        );
        Ok(())
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
    host_key_via(host, port, None).await
}

pub(crate) async fn host_key_via(
    host: &str,
    port: u16,
    proxy: Option<&ProxySettings>,
) -> Result<(String, String)> {
    let seen = Arc::new(Mutex::new(None));
    let probe = async {
        let handler = KeyProbe(seen.clone());
        if let Some(proxy) = proxy {
            let stream = proxy.connect(host, port).await?;
            client::connect_stream(Arc::default(), stream, handler).await
        } else {
            client::connect(Arc::default(), (host, port), handler).await
        }
    };
    if let Ok(Err(error)) = timeout(SETUP, probe).await
        && seen.lock().unwrap().is_none()
    {
        return Err(error);
    }
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
    jump: Option<Box<Connection>>,
    remote_routes: Arc<RemoteRoutes>,
}

impl Connection {
    /// `secret` yields the password, or the passphrase of an encrypted identity file.
    /// It is only consulted when the credential cannot be used without it.
    pub async fn connect(
        host: &Host,
        secret: &(dyn Fn() -> Option<String> + Sync),
        jump: Option<Self>,
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
        if jump.is_some() && host.proxy.is_some() {
            return Err(invalid("choose an SSH jump host or an outbound proxy"));
        }
        let failed = |message| Error::new(ErrorCode::AuthenticationFailed, message);
        if host.ssh_agent && host.identity_file.is_some() {
            return Err(invalid(
                "select either a private key file or the local SSH agent",
            ));
        }
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
            let remote_routes = Arc::<RemoteRoutes>::default();
            let pinned = Pinned {
                fingerprint: host.host_key_sha256.clone(),
                remote_routes: remote_routes.clone(),
            };
            let config = Arc::new(config);
            let mut handle = if let Some(jump) = &jump {
                let stream = jump.tcp_channel(&host.host, host.port).await?.into_stream();
                client::connect_stream(config, stream, pinned).await?
            } else if let Some(proxy) = &host.proxy {
                let stream = proxy.connect(&host.host, host.port).await?;
                client::connect_stream(config, stream, pinned).await?
            } else {
                client::connect(config, (host.host.as_str(), host.port), pinned).await?
            };
            if host.ssh_agent {
                let mut agent = AgentClient::connect_env()
                    .await
                    .map_err(|_| failed("本地 SSH 密钥代理不可用，请检查 SSH_AUTH_SOCK。"))?;
                if let Some(binding) = handle.agent_session_binding() {
                    let _ = agent.bind_session(binding, false).await;
                }
                let identities = agent
                    .request_identities()
                    .await
                    .map_err(|_| failed("无法读取本地 SSH 密钥代理的公钥。"))?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                for key in identities
                    .into_iter()
                    .filter_map(|identity| match identity {
                        AgentIdentity::PublicKey { key, .. } => Some(key),
                        AgentIdentity::Certificate { .. } => None,
                    })
                    .take(6)
                {
                    if handle
                        .authenticate_publickey_with(&host.username, key, hash, &mut agent)
                        .await
                        .map_err(|_| failed("本地 SSH 密钥代理未完成签名。"))?
                        .success()
                    {
                        return Ok(Self {
                            handle,
                            jump: jump.map(Box::new),
                            remote_routes,
                        });
                    }
                }
                return Err(failed("服务器未接受本地 SSH 密钥代理的公钥。"));
            }
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
            Ok(Self {
                handle,
                jump: jump.map(Box::new),
                remote_routes,
            })
        };
        timeout(SETUP, connect).await.map_err(|_| {
            Error::new(
                ErrorCode::ConnectionFailed,
                "connection or authentication timed out",
            )
        })?
    }

    pub fn is_closed(&self) -> bool {
        std::iter::successors(Some(self), |connection| connection.jump.as_deref())
            .any(|connection| connection.handle.is_closed())
    }

    pub(crate) async fn tcp_channel(
        &self,
        target: &str,
        port: u16,
    ) -> Result<Channel<client::Msg>> {
        Ok(timeout(
            SETUP,
            self.handle
                .channel_open_direct_tcpip(target, u32::from(port), "127.0.0.1", 0),
        )
        .await
        .map_err(|_| Error::new(ErrorCode::ConnectionFailed, "SSH TCP channel timed out"))??)
    }

    pub(crate) async fn probe_through(&self, host: &str, port: u16) -> Result<(String, String)> {
        let seen = Arc::new(Mutex::new(None));
        let channel = self.tcp_channel(host, port).await?;
        let _ = timeout(
            SETUP,
            client::connect_stream(
                Arc::default(),
                channel.into_stream(),
                KeyProbe(seen.clone()),
            ),
        )
        .await;
        let result = seen.lock().unwrap().take();
        result
            .ok_or_else(|| Error::new(ErrorCode::ConnectionFailed, "无法通过跳板机读取目标指纹。"))
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
        for connection in std::iter::successors(Some(self), |connection| connection.jump.as_deref())
        {
            connection.remote_routes.clear();
            let _ = connection
                .handle
                .disconnect(russh::Disconnect::ByApplication, "space closed", "en")
                .await;
        }
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
