//! Desktop file and monitoring operations. No device paths are exposed on the agent socket.
use super::*;
#[path = "editor.rs"]
pub(super) mod editor;
pub(super) struct FileSession {
    session: RawSftpSession,
    editable: bool,
}
impl std::ops::Deref for FileSession {
    type Target = RawSftpSession;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncWrite, AsyncWriteExt};

#[derive(Serialize)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub size: Option<u64>,
    pub modified: Option<u32>,
    pub permissions: Option<u32>,
}
#[derive(Serialize)]
pub struct Directory {
    pub path: String,
    pub entries: Vec<RemoteEntry>,
    pub truncated: bool,
}
#[derive(Clone, Serialize)]
pub struct TransferProgress {
    pub done: u64,
    pub total: Option<u64>,
}
fn sftp_error(error: SftpError) -> Error {
    Error::new(
        ErrorCode::InvalidRequest,
        match &error {
            SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile => {
                "目录或文件不存在，请刷新后重试。".into()
            }
            SftpError::Status(status) if status.status_code == StatusCode::PermissionDenied => {
                "SSH 账号没有该文件的操作权限。".into()
            }
            _ => format!("文件操作未成功：{error}。请检查权限、同名项目和连接状态。"),
        },
    )
}
fn path_valid(path: &str) -> Result<()> {
    if path.is_empty() || path.contains('\0') || path.len() > 4096 {
        return err(
            ErrorCode::InvalidRequest,
            "path must be nonempty, NUL-free and at most 4096 bytes",
        );
    }
    Ok(())
}
fn stopped(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        err(
            ErrorCode::InvalidRequest,
            "传输已取消；未完成的远端临时文件可能保留，请刷新目录核实。",
        )
    } else {
        Ok(())
    }
}

impl Space {
    async fn sftp(&self) -> Result<Arc<FileSession>> {
        let mut slot = self.files.lock().await;
        if self.conn.is_closed() {
            return err(
                ErrorCode::ConnectionFailed,
                "connection closed; reconnect explicitly",
            );
        }
        if let Some(session) = &*slot {
            return Ok(session.clone());
        }
        let (session, editable) = self.conn.sftp().await?;
        let session = Arc::new(FileSession { session, editable });
        *slot = Some(session.clone());
        Ok(session)
    }
    async fn generation(&self, by: By<'_>) -> Result<String> {
        let control = self.control.lock().await;
        check(&control, &by)?;
        Ok(control.lease.clone())
    }
    // A queued human transfer also loses its authority after give-back and takeover.
    async fn file_write<T>(
        &self,
        by: By<'_>,
        generation: &str,
        work: impl Future<Output = std::result::Result<T, SftpError>>,
    ) -> Result<T> {
        let control = self.control.lock().await;
        check(&control, &by)?;
        if control.lease != generation {
            return err(
                ErrorCode::StaleLease,
                "控制权已变化，请核实传输结果后重新操作。",
            );
        }
        tokio::time::timeout(SUBMIT, work)
            .await
            .map_err(|_| {
                Error::new(
                    ErrorCode::DeliveryUnknown,
                    "文件操作超时，远端结果需要核实；不会自动重试。",
                )
            })?
            .map_err(sftp_error)
    }
}

impl Service {
    /// Human observation is allowed while an agent holds the space. Directory replies are bounded.
    pub async fn files(&self, space: &str, path: &str) -> Result<Directory> {
        path_valid(path)?;
        let space = self.find_space(space, false)?;
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "文件操作正在进行，请稍后刷新。"))?;
        let sftp = space.sftp().await?;
        let names = sftp.realpath(path).await.map_err(sftp_error)?;
        let path = names
            .files
            .first()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidRequest,
                    "server returned no canonical path",
                )
            })?
            .filename
            .clone();
        path_valid(&path)?;
        let handle = sftp.opendir(path.clone()).await.map_err(sftp_error)?.handle;
        let result = async {
            let mut entries = Vec::new();
            let mut bytes = 0;
            let mut truncated = false;
            'directory: loop {
                let batch = match sftp.readdir(&handle).await {
                    Ok(batch) => batch,
                    Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                        break;
                    }
                    Err(error) => return Err(sftp_error(error)),
                };
                if batch.files.is_empty() {
                    return err(
                        ErrorCode::InvalidRequest,
                        "server returned an empty directory page without EOF",
                    );
                }
                for file in batch.files {
                    let name = file.filename;
                    if name == "." || name == ".." {
                        continue;
                    }
                    if name.contains('/') || name.contains('\0') {
                        continue;
                    }
                    bytes += name.len() * 2 + path.len() + 128;
                    if entries.len() >= 5000 || bytes > 512 * 1024 {
                        truncated = true;
                        break 'directory;
                    }
                    let attrs = file.attrs;
                    entries.push(RemoteEntry {
                        path: format!("{}/{}", path.trim_end_matches('/'), name),
                        name,
                        kind: if attrs.file_type().is_dir() {
                            "directory"
                        } else if attrs.file_type().is_symlink() {
                            "symlink"
                        } else if attrs.file_type().is_file() {
                            "file"
                        } else {
                            "other"
                        },
                        size: attrs.size,
                        modified: attrs.mtime,
                        permissions: attrs.permissions,
                    });
                }
            }
            entries.sort_by(|a, b| {
                (a.kind != "directory", a.name.to_lowercase())
                    .cmp(&(b.kind != "directory", b.name.to_lowercase()))
            });
            Ok(Directory {
                path,
                entries,
                truncated,
            })
        }
        .await;
        let _ = sftp.close(handle).await;
        result
    }

    /// Upload a regular file without replacing existing content. Data stays outside the webview.
    pub async fn upload_file<R: AsyncRead + Unpin>(
        &self,
        by: By<'_>,
        remote: &str,
        mut source: R,
        total: u64,
        cancel: &AtomicBool,
        progress: impl Fn(TransferProgress),
    ) -> Result<()> {
        path_valid(remote)?;
        if !remote.starts_with('/') {
            return err(
                ErrorCode::InvalidRequest,
                "absolute upload destination required",
            );
        }
        let space = self.space_of(&by)?;
        let generation = space.generation(by).await?;
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let sftp = space.sftp().await?;
        let parent = remote
            .rsplit_once('/')
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "absolute destination required"))?
            .0;
        let temporary = format!("{parent}/.vibe-upload-{}", token(16));
        stopped(cancel)?;
        let handle = space
            .file_write(
                by,
                &generation,
                sftp.open(
                    temporary.clone(),
                    OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                    FileAttributes {
                        permissions: Some(0o600),
                        ..Default::default()
                    },
                ),
            )
            .await?
            .handle;
        let result = async {
            let mut done = 0;
            let mut buffer = vec![0; MAX_INPUT];
            loop {
                stopped(cancel)?;
                let n = tokio::time::timeout(UPLOAD_IDLE, source.read(&mut buffer))
                    .await
                    .map_err(|_| Error::new(ErrorCode::InvalidRequest, "读取源文件超时"))?
                    .map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?;
                if n == 0 {
                    break;
                }
                space
                    .file_write(
                        by,
                        &generation,
                        sftp.write(&handle, done, buffer[..n].to_vec()),
                    )
                    .await?;
                done += n as u64;
                progress(TransferProgress {
                    done,
                    total: Some(total),
                });
            }
            if done != total {
                return err(
                    ErrorCode::InvalidRequest,
                    "源文件大小已变化，未发布上传结果。请核实后重试。",
                );
            }
            stopped(cancel)?;
            space
                .file_write(by, &generation, sftp.close(&handle))
                .await?;
            // SFTP v3 rename fails if destination exists, including a symlink. Never unlink it.
            space
                .file_write(by, &generation, sftp.rename(temporary.clone(), remote))
                .await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            // Closing an open handle releases resources; do not remove remote data after revocation.
            let _ = sftp.close(handle).await;
        }
        result.map_err(|mut e| {
            e.message = format!(
                "{} 未发布的内容可能位于 {temporary}；同名目标不会被覆盖。",
                e.message
            );
            e
        })
    }

    pub async fn download_file<W: AsyncWrite + Unpin>(
        &self,
        space: &str,
        remote: &str,
        mut target: W,
        cancel: &AtomicBool,
        progress: impl Fn(TransferProgress),
    ) -> Result<()> {
        path_valid(remote)?;
        let space = self.find_space(space, false)?;
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let sftp = space.sftp().await?;
        let attrs = sftp.lstat(remote).await.map_err(sftp_error)?.attrs;
        if !attrs.file_type().is_file() {
            return err(
                ErrorCode::InvalidRequest,
                "请选择普通文件；不自动跟随符号链接或读取设备文件。",
            );
        }
        let handle = sftp
            .open(remote, OpenFlags::READ, FileAttributes::default())
            .await
            .map_err(sftp_error)?
            .handle;
        let result = async {
            let before = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
            if !before.file_type().is_file() {
                return err(ErrorCode::InvalidRequest, "文件类型已变化");
            }
            let mut done = 0;
            loop {
                stopped(cancel)?;
                let data = match sftp.read(&handle, done, MAX_INPUT as u32).await {
                    Ok(data) => data.data,
                    Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                        break;
                    }
                    Err(error) => return Err(sftp_error(error)),
                };
                if data.is_empty() {
                    return err(ErrorCode::ConnectionFailed, "empty SFTP data without EOF");
                }
                target
                    .write_all(&data)
                    .await
                    .map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?;
                done += data.len() as u64;
                progress(TransferProgress {
                    done,
                    total: before.size,
                });
            }
            let after = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
            if before.size != Some(done) || before.size != after.size || before.mtime != after.mtime
            {
                return err(
                    ErrorCode::InvalidRequest,
                    "下载期间远端文件已变化，请重新核实。",
                );
            }
            target
                .flush()
                .await
                .map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?;
            Ok(())
        }
        .await;
        let _ = sftp.close(handle).await;
        result
    }

    /// Nonrecursive operations; deleting a nonempty directory fails at the server.
    pub async fn file_action(
        &self,
        by: By<'_>,
        action: &str,
        path: &str,
        destination: Option<&str>,
    ) -> Result<()> {
        path_valid(path)?;
        if let Some(destination) = destination {
            path_valid(destination)?;
        }
        let space = self.space_of(&by)?;
        let generation = space.generation(by).await?;
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let sftp = space.sftp().await?;
        match action {
            "mkdir" => {
                space
                    .file_write(by, &generation, sftp.mkdir(path, FileAttributes::default()))
                    .await?;
            }
            "rename" => {
                space
                    .file_write(
                        by,
                        &generation,
                        sftp.rename(
                            path,
                            destination.ok_or_else(|| {
                                Error::new(ErrorCode::InvalidRequest, "destination required")
                            })?,
                        ),
                    )
                    .await?;
            }
            "delete" => {
                let attrs = sftp.lstat(path).await.map_err(sftp_error)?.attrs;
                if attrs.file_type().is_dir() {
                    space.file_write(by, &generation, sftp.rmdir(path)).await?;
                } else {
                    space.file_write(by, &generation, sftp.remove(path)).await?;
                }
            }
            _ => return err(ErrorCode::InvalidRequest, "unknown file action"),
        }
        Ok(())
    }

    /// Fixed read-only Linux probe, one channel, no installation and no sudo. The page owns polling.
    pub async fn resources(&self, space: &str) -> Result<Value> {
        let space = self.find_space(space, false)?;
        let _sample = space.monitoring.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "resource sample already in flight",
            )
        })?;
        let mut channel = space.conn.channel(None).await?;
        channel.exec(true, include_str!("resources.sh")).await?;
        let result = tokio::time::timeout(Duration::from_secs(12), async {
            let mut bytes = Vec::new();
            let mut code = None;
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        if bytes.len() + data.len() > 65536 {
                            return err(ErrorCode::ResourceLimit, "resource response too large");
                        }
                        bytes.extend_from_slice(&data);
                    }
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            if code != Some(0) {
                return err(
                    ErrorCode::InvalidRequest,
                    "资源监控需要 Linux /proc、awk 和 df；采样未成功。终端和文件功能仍可使用。",
                );
            }
            serde_json::from_slice(&bytes).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidRequest,
                    "服务器返回了无法识别的资源采样数据。",
                )
            })
        })
        .await;
        let _ = channel.close().await;
        result.map_err(|_| Error::new(ErrorCode::ConnectionFailed, "资源采样超时，请检查连接。"))?
    }
}
