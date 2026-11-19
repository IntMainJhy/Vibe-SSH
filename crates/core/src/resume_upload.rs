use super::*;
use std::{
    fs::{Metadata, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SourceVersion {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl SourceVersion {
    fn new(metadata: &Metadata) -> Result<Self> {
        if !metadata.is_file() {
            return err(ErrorCode::InvalidRequest, "请选择普通文件上传。");
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct UploadPlan {
    pub host_id: String,
    pub remote: String,
    host: String,
    port: u16,
    username: String,
    fingerprint: String,
    source: PathBuf,
    version: SourceVersion,
    stage_key: String,
    #[serde(skip)]
    generation: Option<String>,
}

impl UploadPlan {
    pub fn size(&self) -> u64 {
        self.version.size
    }

    pub fn name(&self) -> &str {
        self.remote.rsplit('/').next().unwrap_or_default()
    }

    fn matches(&self, host: &Host) -> bool {
        self.host_id == host.id
            && self.host == host.host
            && self.port == host.port
            && self.username == host.username
            && self.fingerprint == host.host_key_sha256
    }

    pub fn staging_path(&self) -> Result<String> {
        if self.stage_key.len() != 32 || !self.stage_key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return err(ErrorCode::InvalidRequest, "上传记录已损坏。");
        }
        Ok(format!(
            "{}/.vibe-upload-{}",
            parent(&self.remote)?.trim_end_matches('/'),
            self.stage_key
        ))
    }
}

pub(super) fn parent(path: &str) -> Result<&str> {
    path_valid(path)?;
    if !path.starts_with('/')
        || path.contains("//")
        || path.split('/').any(|part| [".", ".."].contains(&part))
    {
        return err(ErrorCode::InvalidRequest, "上传需要可校验的绝对路径。");
    }
    match path.rsplit_once('/') {
        Some((_, "")) | None => err(ErrorCode::InvalidRequest, "上传目标须包含文件名。"),
        Some(("", _)) => Ok("/"),
        Some((directory, _)) => Ok(directory),
    }
}

fn open_source(path: &Path) -> Result<std::fs::File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?)
}

pub(super) async fn verify_parent(session: &FileSession, parent: &str) -> Result<()> {
    let actual = session.realpath(parent).await.map_err(sftp_error)?;
    if actual
        .files
        .first()
        .is_none_or(|entry| entry.filename != parent)
    {
        return err(ErrorCode::FileChanged, "服务器上传目录已变化，请重新选择。");
    }
    Ok(())
}

pub(super) async fn unused_destination(session: &FileSession, remote: &str) -> Result<()> {
    match session.lstat(remote).await {
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => Ok(()),
        Err(error) => Err(sftp_error(error)),
        Ok(_) => err(
            ErrorCode::FileChanged,
            "目标文件已经存在。请核实服务器内容，原文件会保留。",
        ),
    }
}

impl Service {
    pub async fn plan_upload(
        &self,
        space: &str,
        directory: &str,
        source: PathBuf,
    ) -> Result<UploadPlan> {
        path_valid(directory)?;
        if !directory.starts_with('/') {
            return err(ErrorCode::InvalidRequest, "请选择服务器上的绝对目录。");
        }
        let by = By::Human(space);
        let space = self.space_of(&by)?;
        let generation = space.generation(by).await?;
        let (source, name, version) = tokio::task::spawn_blocking(move || {
            let file = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "文件名不是有效的 UTF-8。"))?;
            let directory = source
                .parent()
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "上传源路径无效。"))?;
            let path = directory.canonicalize()?.join(file);
            let version = SourceVersion::new(&open_source(&path)?.metadata()?)?;
            Ok::<_, Error>((path, file.to_owned(), version))
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal, "本地文件检查未完成。"))??;
        let sftp = space.sftp().await?;
        let remote = format!("{}/{name}", directory.trim_end_matches('/'));
        verify_parent(&sftp, parent(&remote)?).await?;
        unused_destination(&sftp, &remote).await?;
        let host = &space.host;
        Ok(UploadPlan {
            host_id: host.id.clone(),
            remote,
            host: host.host.clone(),
            port: host.port,
            username: host.username.clone(),
            fingerprint: host.host_key_sha256.clone(),
            source,
            version,
            stage_key: token(32),
            generation: Some(generation),
        })
    }

    pub async fn resume_upload(
        &self,
        space: &str,
        plan: &UploadPlan,
        resume: bool,
        cancel: &AtomicBool,
        progress: impl Fn(TransferProgress),
    ) -> Result<()> {
        let by = By::Human(space);
        let space = self.space_of(&by)?;
        if !plan.matches(&space.host) {
            return err(ErrorCode::InvalidRequest, "请连接上传记录对应的服务器。");
        }
        let generation = space.generation(by).await?;
        if !resume && plan.generation.as_ref() != Some(&generation) {
            return err(ErrorCode::StaleLease, "控制权已变化，请重新选择上传文件。");
        }
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let path = plan.source.clone();
        let source = tokio::task::spawn_blocking(move || open_source(&path))
            .await
            .map_err(|_| Error::new(ErrorCode::Internal, "本地文件打开未完成。"))??;
        let sftp = space.sftp().await?;
        let directory = parent(&plan.remote)?;
        let staging = plan.staging_path()?;
        verify_parent(&sftp, directory).await?;
        unused_destination(&sftp, &plan.remote).await?;
        let mut source = tokio::fs::File::from_std(source);
        if SourceVersion::new(&source.metadata().await?)? != plan.version {
            return err(ErrorCode::FileChanged, "本地文件已变化，请重新选择上传。");
        }
        stopped(cancel)?;
        let flags = OpenFlags::READ | OpenFlags::WRITE;
        let attrs = if resume {
            Some(regular_file(&sftp, &staging).await?)
        } else {
            None
        };
        let flags = if resume {
            flags
        } else {
            flags | OpenFlags::CREATE | OpenFlags::EXCLUDE
        };
        let handle = space
            .file_write(
                by,
                &generation,
                sftp.open(
                    &staging,
                    flags,
                    FileAttributes {
                        permissions: Some(0o600),
                        ..Default::default()
                    },
                ),
            )
            .await?
            .handle;
        let result = async {
            let staged = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
            let offset = staged
                .size
                .filter(|&size| {
                    if resume {
                        size <= plan.size()
                    } else {
                        size == 0
                    }
                })
                .filter(|_| {
                    staged.file_type().is_file()
                        && staged
                            .permissions
                            .is_some_and(|mode| mode & 0o7777 == 0o600)
                        && attrs
                            .as_ref()
                            .is_none_or(|prior| same_file_version(prior, &staged))
                })
                .ok_or_else(|| {
                    Error::new(ErrorCode::FileChanged, "远端临时文件与上传记录不符。")
                })?;
            let mut buffer = vec![0; MAX_INPUT];
            let mut done = 0;
            while done < offset {
                stopped(cancel)?;
                let count = (offset - done).min(MAX_INPUT as u64) as u32;
                let data = read_download(&sftp, &handle, done, count)
                    .await?
                    .ok_or_else(|| Error::new(ErrorCode::FileChanged, "远端临时文件已缩短。"))?;
                source.read_exact(&mut buffer[..data.len()]).await?;
                if buffer[..data.len()] != data {
                    return err(
                        ErrorCode::FileChanged,
                        "远端已上传字节与本地文件不一致。请核实临时文件。",
                    );
                }
                done += data.len() as u64;
            }
            progress(TransferProgress {
                done,
                total: Some(plan.size()),
            });
            while done < plan.size() {
                stopped(cancel)?;
                let len = (plan.size() - done).min(MAX_INPUT as u64) as usize;
                source.read_exact(&mut buffer[..len]).await?;
                space
                    .file_write(
                        by,
                        &generation,
                        sftp.write(&handle, done, buffer[..len].to_vec()),
                    )
                    .await?;
                done += len as u64;
                progress(TransferProgress {
                    done,
                    total: Some(plan.size()),
                });
            }
            stopped(cancel)?;
            let actual = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
            if SourceVersion::new(&source.metadata().await?)? != plan.version
                || actual.size != Some(plan.size())
                || !same_file_version(&actual, &regular_file(&sftp, &staging).await?)
            {
                return err(
                    ErrorCode::FileChanged,
                    "源文件或服务器临时文件已变化，请核实后重新上传。",
                );
            }
            verify_parent(&sftp, directory).await?;
            space
                .file_write(by, &generation, sftp.close(&handle))
                .await?;
            stopped(cancel)?;
            space
                .file_write(by, &generation, sftp.rename(&staging, &plan.remote))
                .await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = sftp.close(handle).await;
        }
        result.map_err(|mut error| {
            error
                .message
                .push_str(&format!(" 服务器临时路径：{staging}。"));
            error
        })
    }
}
