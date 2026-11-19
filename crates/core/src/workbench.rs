//! Remote files and monitoring. The agent socket exposes only bounded reads.
use super::*;
#[path = "directory_apply.rs"]
mod directory_apply;
pub use directory_apply::DiffApplyResult;
#[path = "directory_diff.rs"]
mod directory_diff;
pub use directory_diff::DiffApplyPlan;
#[path = "editor.rs"]
pub(super) mod editor;
#[path = "folder.rs"]
mod folder;
pub use folder::FolderPlan;
#[path = "resume_upload.rs"]
mod resume_upload;
pub use resume_upload::UploadPlan;
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
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use vibe_ssh_protocol::{
    ErrorBody, FileTransferRecord, MAX_FILE_ENTRIES, MountUsage, ResourceSample,
};

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

pub struct TransferObservation {
    record: HistoryRecord,
    started: Instant,
}

impl Service {
    /// Desktop-only observation, with human visibility even inside a taken-over agent space.
    pub async fn observe_transfer(
        &self,
        id: &str,
        transfer: FileTransferRecord,
    ) -> Option<TransferObservation> {
        path_valid(&transfer.remote).ok()?;
        let space = self.find_space(id, false).ok()?;
        let record = HistoryRecord {
            record: format!("h{}", token(32)),
            host: space.host.id.clone(),
            host_name: short(&space.host.name, 256).0,
            space_name: short(&space.name, 256).0,
            actor: "desktop".into(),
            origin: Owner::Human,
            command: None,
            cwd: None,
            command_truncated: false,
            pty: false,
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            elapsed_ms: 0,
            status: JobStatus::Running,
            reason: None,
            exit_code: None,
            exit_signal: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            transfer: Some(transfer),
            deployment: Vec::new(),
            archive: None,
        };
        let started = Instant::now();
        self.history
            .save(record.clone(), vec![], vec![])
            .await
            .ok()?;
        Some(TransferObservation { record, started })
    }

    pub async fn finish_transfer(
        &self,
        observation: TransferObservation,
        error: Option<&ErrorBody>,
    ) {
        let TransferObservation {
            mut record,
            started,
        } = observation;
        record.elapsed_ms = started.elapsed().as_millis() as u64;
        let stderr = match error {
            Some(error) => {
                record.status = if matches!(
                    error.code,
                    ErrorCode::DeliveryUnknown | ErrorCode::ConnectionFailed
                ) {
                    JobStatus::Unknown
                } else {
                    JobStatus::Failed
                };
                record.stderr_bytes = error.message.len() as u64;
                let mut start = error.message.len().saturating_sub(OUTPUT);
                while !error.message.is_char_boundary(start) {
                    start += 1;
                }
                error.message.as_bytes()[start..].to_vec()
            }
            None => {
                record.status = JobStatus::Succeeded;
                Vec::new()
            }
        };
        // The attempt's real result takes precedence over any local journal I/O error.
        let _ = self.history.save(record, vec![], stderr).await;
    }
}

#[derive(Serialize, Deserialize)]
pub struct DownloadPlan {
    pub host_id: String,
    pub remote: String,
    pub size: u64,
    host: String,
    port: u16,
    username: String,
    fingerprint: String,
    modified: Option<u32>,
}

impl DownloadPlan {
    pub fn matches(&self, host: &Host) -> bool {
        self.host_id == host.id
            && self.host == host.host
            && self.port == host.port
            && self.username == host.username
            && self.fingerprint == host.host_key_sha256
    }
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
            .map_err(|error| match &error {
                SftpError::Status(status)
                    if !matches!(
                        status.status_code,
                        StatusCode::ConnectionLost | StatusCode::NoConnection
                    ) =>
                {
                    sftp_error(error)
                }
                SftpError::Limited(_) => sftp_error(error),
                _ => Error::new(
                    ErrorCode::DeliveryUnknown,
                    format!("文件操作缺少服务器回执：{error}。请核实远端状态。"),
                ),
            })
    }
}

struct FileOperation<'a> {
    space: &'a Space,
    session: &'a FileSession,
    by: By<'a>,
    generation: &'a str,
}

impl Service {
    /// Human observation is allowed while an agent holds the space. Directory replies are bounded.
    pub async fn files(&self, space: &str, path: &str) -> Result<Directory> {
        self.list_files(space, path, false, 5000, 512 * 1024).await
    }

    pub(crate) async fn agent_files(
        &self,
        space: &str,
        path: &str,
        limit: usize,
    ) -> Result<Directory> {
        if !(1..=MAX_FILE_ENTRIES).contains(&limit) {
            return err(
                ErrorCode::InvalidRequest,
                "file list limit must be 1..=1000",
            );
        }
        self.list_files(space, path, true, limit, 64 * 1024).await
    }

    async fn list_files(
        &self,
        space: &str,
        path: &str,
        agent: bool,
        limit: usize,
        budget: usize,
    ) -> Result<Directory> {
        path_valid(path)?;
        let space = self.remote_read_space(space, agent)?;
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
        let result = tokio::time::timeout(Duration::from_secs(15), async {
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
                    if entries.len() >= limit || bytes > budget {
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
        })
        .await;
        let _ = sftp.close(handle).await;
        result.map_err(|_| Error::new(ErrorCode::ConnectionFailed, "directory read timed out"))?
    }

    /// Upload a regular file without replacing existing content. Data stays outside the webview.
    pub async fn upload_file<R: AsyncRead + Unpin>(
        &self,
        by: By<'_>,
        remote: &str,
        source: R,
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
        FileOperation {
            space: &space,
            session: &sftp,
            by,
            generation: &generation,
        }
        .upload(remote, source, total, cancel, progress)
        .await
    }
}

impl FileOperation<'_> {
    async fn upload<R: AsyncRead + Unpin>(
        &self,
        remote: &str,
        mut source: R,
        total: u64,
        cancel: &AtomicBool,
        progress: impl Fn(TransferProgress),
    ) -> Result<()> {
        path_valid(remote)?;
        let Self {
            space,
            session: sftp,
            by,
            generation,
        } = *self;
        let parent = remote
            .rsplit_once('/')
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "absolute destination required"))?
            .0;
        let temporary = format!("{parent}/.vibe-upload-{}", token(16));
        stopped(cancel)?;
        let handle = space
            .file_write(
                by,
                generation,
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
                        generation,
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
                .file_write(by, generation, sftp.close(&handle))
                .await?;
            // SFTP v3 rename fails if destination exists, including a symlink. Never unlink it.
            space
                .file_write(by, generation, sftp.rename(temporary.clone(), remote))
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
}

impl Service {
    pub async fn plan_download(&self, space: &str, remote: &str) -> Result<DownloadPlan> {
        path_valid(remote)?;
        let space = self.find_space(space, false)?;
        let sftp = space.sftp().await?;
        let attrs = regular_file(&sftp, remote).await?;
        let host = &space.host;
        Ok(DownloadPlan {
            host_id: host.id.clone(),
            remote: remote.into(),
            size: attrs
                .size
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "服务器没有提供文件大小。"))?,
            host: host.host.clone(),
            port: host.port,
            username: host.username.clone(),
            fingerprint: host.host_key_sha256.clone(),
            modified: attrs.mtime,
        })
    }

    pub async fn resume_download(
        &self,
        space: &str,
        plan: &DownloadPlan,
        target: &mut tokio::fs::File,
        cancel: &AtomicBool,
        progress: impl Fn(TransferProgress),
    ) -> Result<()> {
        path_valid(&plan.remote)?;
        let space = self.find_space(space, false)?;
        if !plan.matches(&space.host) {
            return err(ErrorCode::InvalidRequest, "请连接下载记录对应的服务器。");
        }
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let sftp = space.sftp().await?;
        let handle = open_download(&sftp, &plan.remote).await?;
        let result = async {
            let before = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
            let offset = target.metadata().await?.len();
            if !before.file_type().is_file()
                || before.size != Some(plan.size)
                || before.mtime != plan.modified
                || offset > plan.size
            {
                return err(
                    ErrorCode::InvalidRequest,
                    "文件与下载开始时的版本不一致，请重新下载。",
                );
            }
            target.rewind().await?;
            let mut matched = 0;
            let mut local = vec![0; MAX_INPUT];
            while matched < offset {
                stopped(cancel)?;
                let count = (offset - matched).min(MAX_INPUT as u64) as u32;
                let data = read_download(&sftp, &handle, matched, count)
                    .await?
                    .ok_or_else(|| {
                        Error::new(ErrorCode::InvalidRequest, "远端文件已缩短，请重新下载。")
                    })?;
                target.read_exact(&mut local[..data.len()]).await?;
                if local[..data.len()] != data {
                    return err(
                        ErrorCode::InvalidRequest,
                        "已下载内容校验失败，请重新下载。",
                    );
                }
                matched += data.len() as u64;
            }
            progress(TransferProgress {
                done: offset,
                total: before.size,
            });
            write_download(&sftp, &handle, before, offset, target, cancel, progress).await
        }
        .await;
        let _ = sftp.close(handle).await;
        result
    }

    pub async fn download_file<W: AsyncWrite + Unpin>(
        &self,
        space: &str,
        remote: &str,
        target: W,
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
        download(&sftp, remote, target, cancel, progress).await
    }
}

async fn download<W: AsyncWrite + Unpin>(
    sftp: &FileSession,
    remote: &str,
    target: W,
    cancel: &AtomicBool,
    progress: impl Fn(TransferProgress),
) -> Result<()> {
    path_valid(remote)?;
    let handle = open_download(sftp, remote).await?;
    let result = async {
        let before = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
        if !before.file_type().is_file() {
            return err(ErrorCode::InvalidRequest, "文件类型已变化");
        }
        write_download(sftp, &handle, before, 0, target, cancel, progress).await
    }
    .await;
    let _ = sftp.close(handle).await;
    result
}

fn same_file_version(a: &FileAttributes, b: &FileAttributes) -> bool {
    a.file_type().is_file()
        && b.file_type().is_file()
        && (a.size, a.mtime, a.permissions, a.uid, a.gid)
            == (b.size, b.mtime, b.permissions, b.uid, b.gid)
}

async fn regular_file(sftp: &FileSession, remote: &str) -> Result<FileAttributes> {
    let attrs = sftp.lstat(remote).await.map_err(sftp_error)?.attrs;
    if !attrs.file_type().is_file() {
        return err(
            ErrorCode::InvalidRequest,
            "请选择普通文件；不自动跟随符号链接或读取设备文件。",
        );
    }
    Ok(attrs)
}

async fn open_download(sftp: &FileSession, remote: &str) -> Result<String> {
    regular_file(sftp, remote).await?;
    Ok(sftp
        .open(remote, OpenFlags::READ, FileAttributes::default())
        .await
        .map_err(sftp_error)?
        .handle)
}

async fn read_download(
    sftp: &FileSession,
    handle: &str,
    offset: u64,
    count: u32,
) -> Result<Option<Vec<u8>>> {
    match sftp.read(handle, offset, count).await {
        Ok(data) if !data.data.is_empty() && data.data.len() <= count as usize => {
            Ok(Some(data.data))
        }
        Ok(_) => err(ErrorCode::ConnectionFailed, "invalid SFTP read length"),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => Ok(None),
        Err(error) => Err(sftp_error(error)),
    }
}

async fn write_download(
    sftp: &FileSession,
    handle: &str,
    before: FileAttributes,
    mut done: u64,
    mut target: impl AsyncWrite + Unpin,
    cancel: &AtomicBool,
    progress: impl Fn(TransferProgress),
) -> Result<()> {
    loop {
        stopped(cancel)?;
        let Some(data) = read_download(sftp, handle, done, MAX_INPUT as u32).await? else {
            break;
        };
        done += data.len() as u64;
        if before.size.is_some_and(|size| done > size) {
            break;
        }
        target.write_all(&data).await?;
        progress(TransferProgress {
            done,
            total: before.size,
        });
    }
    let after = sftp.fstat(handle).await.map_err(sftp_error)?.attrs;
    if before.size != Some(done) || before.size != after.size || before.mtime != after.mtime {
        return err(
            ErrorCode::InvalidRequest,
            "下载期间远端文件已变化，请重新核实。",
        );
    }
    target.flush().await?;
    Ok(())
}

impl Service {
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
            "chmod" => {
                let mode = destination
                    .filter(|value| {
                        (3..=4).contains(&value.len())
                            && value.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
                    })
                    .and_then(|value| u32::from_str_radix(value, 8).ok())
                    .ok_or_else(|| {
                        Error::new(ErrorCode::InvalidRequest, "权限需填写三到四位八进制数字。")
                    })?;
                let before = sftp.lstat(path).await.map_err(sftp_error)?.attrs;
                if !before.file_type().is_file() {
                    return err(
                        ErrorCode::InvalidRequest,
                        "权限编辑仅支持普通文件，不跟随符号链接。",
                    );
                }
                let handle = sftp
                    .open(path, OpenFlags::READ, FileAttributes::default())
                    .await
                    .map_err(sftp_error)?
                    .handle;
                let result = async {
                    let opened = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
                    if !opened.file_type().is_file()
                        || (
                            opened.size,
                            opened.mtime,
                            opened.permissions,
                            opened.uid,
                            opened.gid,
                        ) != (
                            before.size,
                            before.mtime,
                            before.permissions,
                            before.uid,
                            before.gid,
                        )
                    {
                        return err(ErrorCode::FileChanged, "文件已变化，请刷新后再修改权限。");
                    }
                    space
                        .file_write(
                            by,
                            &generation,
                            sftp.fsetstat(
                                &handle,
                                FileAttributes {
                                    permissions: Some(mode),
                                    ..Default::default()
                                },
                            ),
                        )
                        .await?;
                    Ok(())
                }
                .await;
                let _ = sftp.close(handle).await;
                result?;
            }
            _ => return err(ErrorCode::InvalidRequest, "unknown file action"),
        }
        Ok(())
    }

    /// Fixed read-only Linux probe, one channel, no installation and no sudo. The page owns polling.
    pub async fn resources(&self, space: &str, agent: bool) -> Result<ResourceSample> {
        let space = self.remote_read_space(space, agent)?;
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
            let mut lines = bytes.splitn(2, |byte| *byte == b'\n');
            let mut sample: ResourceSample =
                serde_json::from_slice(lines.next().unwrap_or_default()).map_err(|_| {
                    Error::new(
                        ErrorCode::InvalidRequest,
                        "服务器返回了无法识别的资源采样数据。",
                    )
                })?;
            if let Some(mounts) = lines.next().and_then(|tail| std::str::from_utf8(tail).ok()) {
                for line in mounts.lines() {
                    if sample.mounts.len() == 64 {
                        sample.mounts_truncated = true;
                        break;
                    }
                    let mut rest = line;
                    let mut size = || {
                        rest = rest.trim_start_matches([' ', '\t']);
                        let end = rest.find([' ', '\t']).unwrap_or(rest.len());
                        let value = rest[..end].parse::<u64>().ok()?.checked_mul(1024)?;
                        rest = &rest[end..];
                        Some(value)
                    };
                    if let (Some(total), Some(used), Some(available)) = (size(), size(), size()) {
                        let path = rest.trim_start_matches([' ', '\t']);
                        if path.starts_with('/')
                            && path.len() <= 4096
                            && !path.chars().any(char::is_control)
                        {
                            sample.mounts.push(MountUsage {
                                path: path.into(),
                                total,
                                used,
                                available,
                            });
                        }
                    }
                }
            }
            Ok(sample)
        })
        .await;
        let _ = channel.close().await;
        result.map_err(|_| Error::new(ErrorCode::ConnectionFailed, "资源采样超时，请检查连接。"))?
    }
}
