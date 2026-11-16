use std::{path::PathBuf, sync::Arc, time::Duration};

use russh::{
    ChannelMsg, ChannelReadHalf, client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key},
};
use russh_sftp::client::SftpSession;
use serde::Deserialize;
use tokio::time::{Instant, timeout};
use uuid::Uuid;
use vibe_ssh_protocol::{
    CompletionReason, ExecutionRequest, ExecutionResult, InputLease, JobStatus, MAX_OUTPUT_LIMIT,
};

use crate::{ControlledInput, Error, Result, output::OutputTail};

/// Developer probe configuration. Persistent host management is a later milestone.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: PathBuf,
    pub host_key_sha256: String,
}

struct PinnedHost {
    fingerprint: String,
}

impl client::Handler for PinnedHost {
    type Error = Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        if key.public_key().fingerprint(HashAlg::Sha256).to_string() != self.fingerprint {
            return Err(Error::HostKeyMismatch);
        }
        Ok(true)
    }
}

pub struct SshConnection {
    handle: client::Handle<PinnedHost>,
}

impl SshConnection {
    pub async fn connect(config: &ConnectionConfig) -> Result<Self> {
        if config.host.is_empty() || config.username.is_empty() || config.port == 0 {
            return Err(Error::InvalidRequest(
                "host, username and a nonzero port are required",
            ));
        }
        if !config.host_key_sha256.starts_with("SHA256:") {
            return Err(Error::InvalidRequest(
                "an explicitly verified SHA256 host fingerprint is required",
            ));
        }
        let key = load_secret_key(&config.identity_file, None).map_err(|_| Error::IdentityLoad)?;
        let connect = async {
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                (config.host.as_str(), config.port),
                PinnedHost {
                    fingerprint: config.host_key_sha256.clone(),
                },
            )
            .await?;
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            let authenticated = handle
                .authenticate_publickey(
                    &config.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await?;
            if !authenticated.success() {
                return Err(Error::AuthenticationFailed);
            }
            Ok(Self { handle })
        };
        timeout(Duration::from_secs(15), connect)
            .await
            .map_err(|_| Error::ConnectionTimeout)?
    }

    pub async fn start(&self, request: ExecutionRequest) -> Result<RunningCommand> {
        validate_request(&request)?;
        let mut channel = timeout(Duration::from_secs(15), self.handle.channel_open_session())
            .await
            .map_err(|_| Error::ConnectionTimeout)??;
        if request.pty {
            channel
                .request_pty(true, "xterm-256color", 100, 30, 0, 0, &[])
                .await?;
            await_channel_reply(&mut channel).await?;
        }
        let command = match &request.cwd {
            Some(cwd) => format!("cd {} && {}", quote_posix(cwd), request.command),
            None => request.command.clone(),
        };
        channel.exec(true, command).await?;
        let (reader, writer) = channel.split();
        let (input, initial_lease) = ControlledInput::new(writer);
        Ok(RunningCommand {
            id: Uuid::new_v4(),
            reader,
            input: Arc::new(input),
            initial_lease,
            request,
            started: Instant::now(),
        })
    }

    pub async fn run(&self, request: ExecutionRequest) -> Result<ExecutionResult> {
        self.start(request).await?.collect().await
    }

    /// Low-level SFTP session for M0. Product file policy is implemented in M2.
    pub async fn sftp(&self) -> Result<SftpSession> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        await_channel_reply(&mut channel).await?;
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| Error::Sftp(e.to_string()))
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.handle
            .disconnect(russh::Disconnect::ByApplication, "client shutdown", "en")
            .await?;
        Ok(())
    }
}

pub struct RunningCommand {
    pub id: Uuid,
    pub input: Arc<ControlledInput>,
    pub initial_lease: InputLease,
    reader: ChannelReadHalf,
    request: ExecutionRequest,
    started: Instant,
}

impl RunningCommand {
    pub async fn collect(mut self) -> Result<ExecutionResult> {
        let mut stdout = OutputTail::new(self.request.output_limit);
        let mut stderr = OutputTail::new(self.request.output_limit);
        let mut exit_code = None;
        let mut exit_signal = None;
        let mut observation_expired = false;
        let deadline = self.started + Duration::from_millis(self.request.timeout_ms);
        loop {
            let message = tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    observation_expired = true;
                    break;
                }
                message = self.reader.wait() => message,
            };
            match message {
                Some(ChannelMsg::Data { data }) => stdout.append(&data),
                Some(ChannelMsg::ExtendedData { data, .. }) if self.request.pty => {
                    stdout.append(&data)
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => stderr.append(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    exit_signal = Some(format!("{signal_name:?}"))
                }
                Some(ChannelMsg::Failure) => return Err(Error::ChannelRejected),
                Some(ChannelMsg::Close) | None => break,
                // EOF can precede exit status. Continue until channel closure.
                _ => {}
            }
        }
        // Release the local channel; this does not guarantee termination remotely.
        let _ = self.input.close().await;
        let (status, completion_reason) = if observation_expired {
            (JobStatus::Unknown, CompletionReason::ObservationTimeout)
        } else if exit_signal.is_some() {
            (JobStatus::Failed, CompletionReason::ExitSignal)
        } else if let Some(code) = exit_code {
            (
                if code == 0 {
                    JobStatus::Succeeded
                } else {
                    JobStatus::Failed
                },
                CompletionReason::ExitStatus,
            )
        } else {
            (JobStatus::Unknown, CompletionReason::MissingExitStatus)
        };
        Ok(ExecutionResult {
            job_id: self.id,
            status,
            completion_reason,
            exit_code,
            exit_signal,
            stdout: stdout.snapshot(),
            stderr: stderr.snapshot(),
            output_merged: self.request.pty,
            elapsed_ms: self.started.elapsed().as_millis(),
        })
    }
}

async fn await_channel_reply(channel: &mut russh::Channel<client::Msg>) -> Result<()> {
    timeout(Duration::from_secs(15), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::WindowAdjusted { .. }) => {}
                _ => return Err(Error::ChannelRejected),
            }
        }
    })
    .await
    .map_err(|_| Error::ConnectionTimeout)?
}

fn validate_request(request: &ExecutionRequest) -> Result<()> {
    if request.command.is_empty()
        || request.command.contains('\0')
        || request.command.len() > 1_048_576
    {
        return Err(Error::InvalidRequest(
            "command must be nonempty, NUL-free and at most 1 MiB",
        ));
    }
    if !(1..=3_600_000).contains(&request.timeout_ms) {
        return Err(Error::InvalidRequest(
            "observation timeout must be 1..=3600000 ms",
        ));
    }
    if !(4..=MAX_OUTPUT_LIMIT).contains(&request.output_limit) {
        return Err(Error::InvalidRequest(
            "output budget must be 4..=1048576 bytes per stream",
        ));
    }
    if request
        .cwd
        .as_ref()
        .is_some_and(|p| !p.starts_with('/') || p.contains('\0'))
    {
        return Err(Error::InvalidRequest(
            "cwd must be an absolute NUL-free POSIX path",
        ));
    }
    Ok(())
}

fn quote_posix(value: &str) -> String {
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
