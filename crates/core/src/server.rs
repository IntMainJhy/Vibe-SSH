use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::JoinHandle,
};
use vibe_ssh_protocol::{
    Call, DEFAULT_READ, ErrorBody, ErrorCode, MAX_FRAME, Owner, PROTOCOL_VERSION, Request,
};

use crate::{Archive, By, Error, Result, Service};

/// `$VIBE_SSH_HOME`, or `~/.vibe-ssh`.
pub fn default_home() -> PathBuf {
    std::env::var_os("VIBE_SSH_HOME").map_or_else(
        || PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".vibe-ssh"),
        PathBuf::from,
    )
}

pub fn socket_path(home: &Path) -> PathBuf {
    home.join("agent.sock")
}

/// Binds the agent socket for the current user only, then serves it in the background.
pub async fn serve(service: Arc<Service>) -> Result<JoinHandle<()>> {
    let home = service.home().clone();
    let io =
        |e: std::io::Error| Error::new(ErrorCode::Internal, format!("{}: {e}", home.display()));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&home)
        .map_err(io)?;
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    let path = socket_path(&home);
    if UnixStream::connect(&path).await.is_ok() {
        return Err(Error::new(
            ErrorCode::ServiceUnavailable,
            "another Vibe-SSH service is already running for this home directory",
        ));
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(io)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    let owner = std::fs::metadata(&home).map_err(io)?.uid();
    let slots = Arc::new(Semaphore::new(64));
    Ok(tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            // The 0700 directory already keeps other users out; this is defence in depth.
            if stream.peer_cred().ok().map(|peer| peer.uid()) != Some(owner) {
                continue;
            }
            let (service, slots) = (service.clone(), slots.clone());
            tokio::spawn(async move {
                let slot = slots.try_acquire();
                let _ = handle(&service, stream, slot.is_ok()).await;
            });
        }
    }))
}

async fn handle(service: &Service, stream: UnixStream, admitted: bool) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = Vec::new();
    let mut reader = BufReader::new(read.take(MAX_FRAME as u64));
    let received =
        tokio::time::timeout(Duration::from_secs(10), reader.read_until(b'\n', &mut line)).await;
    let reply = match received {
        _ if !admitted => Err(Error::new(
            ErrorCode::ResourceLimit,
            "too many concurrent requests; retry shortly",
        )),
        Ok(Ok(_)) if line.ends_with(b"\n") => {
            // An upload follows its request line and may be of any size.
            reader.get_mut().set_limit(u64::MAX);
            dispatch(service, &line, &mut reader).await
        }
        _ => Err(Error::new(
            ErrorCode::InvalidRequest,
            "expected one newline-terminated JSON request of at most 2 MiB",
        )),
    };
    let body = match reply {
        Ok(value) => json!({ "ok": value }),
        Err(Error { code, message }) => json!({ "error": ErrorBody { code, message } }),
    };
    write.write_all(format!("{body}\n").as_bytes()).await
}

/// Everything reachable here acts as an agent. The user's operations are not on this socket.
async fn dispatch<A: AsyncRead + Unpin>(
    service: &Service,
    line: &[u8],
    upload: &mut A,
) -> Result<Value> {
    let invalid =
        |e: serde_json::Error| Error::new(ErrorCode::InvalidRequest, format!("request: {e}"));
    let value: Value = serde_json::from_slice(line).map_err(invalid)?;
    if value["v"] != PROTOCOL_VERSION {
        return Err(Error::new(
            ErrorCode::VersionMismatch,
            format!(
                "this service speaks protocol {PROTOCOL_VERSION}; use the vibe-ssh CLI that ships with it and re-read `vibe-ssh skill`"
            ),
        ));
    }
    let Request { call, .. } = serde_json::from_value(value).map_err(invalid)?;
    Ok(match call {
        Call::Status => service.status(),
        Call::Hosts => json!(service.hosts(true)?),
        Call::FilesList {
            space,
            remote,
            limit,
        } => json!(service.agent_files(&space, &remote, limit).await?),
        Call::FilesRead {
            space,
            remote,
            max_bytes,
        } => json!(service.agent_text(&space, &remote, max_bytes).await?),
        Call::Resources { space } => json!(service.resources(&space, true).await?),
        Call::HistoryList { offset, limit } => {
            json!(service.history_list(offset, limit, true).await?)
        }
        Call::HistoryGet { record, tail_bytes } => {
            json!(service.history_get(&record, tail_bytes, true).await?)
        }
        Call::HistoryLog {
            record,
            stream,
            offset,
            limit,
        } => {
            json!(
                service
                    .history_log(&record, stream, offset, limit, true)
                    .await?
            )
        }
        Call::Diff {
            space,
            remote,
            manifest,
        } => json!(service.diff(&space, &remote, manifest, true).await?),
        Call::SpaceOpen { host, name, actor } => {
            json!(service.open(&host, &name, &actor, Owner::Agent).await?)
        }
        Call::SpaceList => json!(service.spaces(true).await),
        Call::SpaceGet { space } => json!(service.space(&space, true).await?),
        Call::SpaceClose { lease } => {
            service.close(By::Lease(&lease)).await?;
            json!({ "closed": true })
        }
        Call::SpaceHandoff { lease, reason } => json!(service.handoff(&lease, &reason).await?),
        Call::DeploymentStart { lease, steps } => {
            json!({"deployment": service.deployment_start(&lease, steps).await?})
        }
        Call::DeploymentStep {
            lease,
            deployment,
            index,
            action,
            wait_ms,
        } => {
            json!(
                service
                    .deployment_step(&lease, &deployment, index, action, wait_ms, upload)
                    .await?
            )
        }
        Call::DeploymentStop {
            lease,
            deployment,
            reason,
        } => {
            json!({"stopped": service.deployment_stop(&lease, &deployment, &reason).await?})
        }
        Call::Run {
            lease,
            command,
            cwd,
            pty,
            wait_ms,
            tail_bytes,
        } => {
            let job = service
                .run(By::Lease(&lease), command, cwd, pty.then_some((120, 40)))
                .await?;
            json!(service.job_settled(&job, wait_ms, tail_bytes, true).await?)
        }
        Call::JobGet { job, tail_bytes } => json!(service.job(&job, tail_bytes, true).await?),
        Call::JobWait {
            cursor,
            max_bytes,
            timeout_ms,
        } => json!(
            service
                .job_wait(&cursor, max_bytes, timeout_ms, true)
                .await?
        ),
        Call::Put {
            lease,
            remote_dir,
            wait_ms,
            tail_bytes,
        } => {
            let job = service.put(By::Lease(&lease), upload, &remote_dir).await?;
            json!(service.job_settled(&job, wait_ms, tail_bytes, true).await?)
        }
        Call::JobInput {
            lease,
            job,
            data,
            end,
        } => {
            service
                .input(By::Lease(&lease), &job, data.as_bytes(), end)
                .await?;
            json!({ "delivered": true })
        }
        Call::JobInterrupt { lease, job } => {
            service.interrupt(By::Lease(&lease), &job).await?;
            json!(service.job(&job, DEFAULT_READ, true).await?)
        }
    })
}

/// Sends one call to the running service. An upload's archive is streamed right after it.
pub async fn request(home: &Path, call: Call, archive: Option<Archive>) -> Result<Value> {
    let unavailable = |_| {
        Error::new(
            ErrorCode::ServiceUnavailable,
            "Vibe-SSH is not running; ask the user to open the Vibe-SSH app (developers: `vibe-ssh serve`)",
        )
    };
    let mut stream = UnixStream::connect(socket_path(home))
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => Error::new(
                ErrorCode::ServiceUnavailable,
                "access to the Vibe-SSH local socket was denied; check the Agent's sandbox or local socket permissions",
            ),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => unavailable(error),
            _ => Error::new(
                ErrorCode::ServiceUnavailable,
                format!("could not connect to the Vibe-SSH local socket: {error}"),
            ),
        })?;
    let mut line = serde_json::to_vec(&Request {
        v: PROTOCOL_VERSION,
        call,
    })
    .expect("requests serialize");
    line.push(b'\n');
    stream.write_all(&line).await.map_err(unavailable)?;
    let sent = match archive {
        Some(archive) => archive.send(&mut stream).await,
        None => Ok(()),
    };
    // Without its closing frame, the end of the stream tells the service the upload was cut.
    let _ = stream.shutdown().await;
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply).await;
    let mut reply: Value = serde_json::from_slice(&reply)
        .map_err(|_| Error::new(ErrorCode::Internal, "malformed reply from the service"))?;
    if let Ok(error) = serde_json::from_value::<ErrorBody>(reply["error"].take()) {
        return Err(Error::new(error.code, error.message));
    }
    sent?;
    Ok(reply["ok"].take())
}
