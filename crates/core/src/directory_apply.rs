use super::directory_diff::{DiffChange, remote_digest};
use super::*;
use crate::diff::{FILE_BYTES, read_selected};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Serialize)]
pub struct DiffApplyResult {
    applied: Vec<AppliedFile>,
    stopped: Option<StoppedFile>,
}

#[derive(Serialize)]
struct AppliedFile {
    path: String,
    backup: Option<String>,
}

#[derive(Serialize)]
struct StoppedFile {
    path: String,
    code: ErrorCode,
    message: String,
}

impl Service {
    pub async fn apply_diff(
        &self,
        plan: DiffApplyPlan,
        project: PathBuf,
        cancel: &AtomicBool,
    ) -> Result<DiffApplyResult> {
        let by = By::Human(&plan.space);
        let space = self.space_of(&by)?;
        if space.generation(by).await? != plan.generation {
            return err(ErrorCode::StaleLease, "控制权已变化，请重新比较。");
        }
        let (plan, contents) = tokio::task::spawn_blocking(move || {
            let contents = plan
                .files
                .iter()
                .map(|change| {
                    let data = read_selected(&project, &change.path, change.size)?;
                    data.filter(|data| {
                        data.len() as u64 == change.size
                            && format!("{:x}", Sha256::digest(data))
                                .eq_ignore_ascii_case(&change.digest)
                    })
                    .ok_or_else(|| changed(&change.path))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok::<_, Error>((plan, contents))
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal, "本地校验任务未完成。"))??;
        let by = By::Human(&plan.space);
        let _operation = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作正在进行。"))?;
        let _replace = self
            .editor_save
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "已有文件正在保存，请稍后再试。"))?;
        let session = space.sftp().await?;
        let operation = FileOperation {
            space: &space,
            session: &session,
            by,
            generation: &plan.generation,
        };
        for change in &plan.files {
            stopped(cancel)?;
            target_path(&session, &plan.root, &change.path, None).await?;
            verify_change(&session, &plan.root, change).await?;
        }
        let mut result = DiffApplyResult {
            applied: Vec::new(),
            stopped: None,
        };
        for (change, data) in plan.files.into_iter().zip(contents) {
            match apply_file(&operation, &plan.root, &change, &data, cancel).await {
                Ok(backup) => result.applied.push(AppliedFile {
                    path: change.path,
                    backup,
                }),
                Err(Error { code, message }) => {
                    result.stopped = Some(StoppedFile {
                        path: change.path,
                        code,
                        message,
                    });
                    break;
                }
            }
        }
        Ok(result)
    }
}

fn changed(path: &str) -> Error {
    Error::new(
        ErrorCode::FileChanged,
        format!("{path} 已变化，请重新比较。"),
    )
}

async fn lstat(session: &FileSession, path: &str) -> Result<Option<FileAttributes>> {
    match session.lstat(path).await {
        Ok(attrs) => Ok(Some(attrs.attrs)),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => Ok(None),
        Err(error) => Err(sftp_error(error)),
    }
}

async fn target_path(
    session: &FileSession,
    root: &str,
    path: &str,
    create: Option<&FileOperation<'_>>,
) -> Result<String> {
    let actual = session.realpath(root).await.map_err(sftp_error)?;
    if actual
        .files
        .first()
        .is_none_or(|file| file.filename != root)
        || lstat(session, root)
            .await?
            .is_none_or(|attrs| !attrs.file_type().is_dir())
    {
        return Err(changed(path));
    }
    let mut full = root.trim_end_matches('/').to_owned();
    if let Some((parent, _)) = path.rsplit_once('/') {
        for part in parent.split('/') {
            full.push('/');
            full.push_str(part);
            path_valid(&full)?;
            match lstat(session, &full).await? {
                Some(attrs) if attrs.file_type().is_dir() => {}
                Some(_) => return Err(changed(path)),
                None => {
                    if let Some(op) = create {
                        op.space
                            .file_write(
                                op.by,
                                op.generation,
                                session.mkdir(
                                    &full,
                                    FileAttributes {
                                        permissions: Some(0o755),
                                        ..Default::default()
                                    },
                                ),
                            )
                            .await?;
                    }
                }
            }
        }
    }
    let full = format!("{}/{}", root.trim_end_matches('/'), path);
    path_valid(&full)?;
    Ok(full)
}

async fn verify_change(session: &FileSession, root: &str, change: &DiffChange) -> Result<()> {
    if let Some(previous) = &change.previous {
        let mut budget = FILE_BYTES;
        let valid = remote_digest(session, root, &change.path, &mut budget)
            .await?
            .is_some_and(|now| {
                now.digest == previous.digest && same_file_version(&now.attrs, &previous.attrs)
            });
        if !valid {
            return Err(changed(&change.path));
        }
        if !session.editable {
            return err(ErrorCode::InvalidRequest, "服务器不支持原子替换和备份。");
        }
        if previous
            .attrs
            .permissions
            .is_none_or(|mode| mode & 0o7000 != 0)
        {
            return err(
                ErrorCode::InvalidRequest,
                "目录应用只替换具有普通权限的文件。",
            );
        }
    } else if lstat(
        session,
        &format!("{}/{}", root.trim_end_matches('/'), change.path),
    )
    .await?
    .is_some()
    {
        return Err(changed(&change.path));
    }
    Ok(())
}

async fn apply_file(
    op: &FileOperation<'_>,
    root: &str,
    change: &DiffChange,
    data: &[u8],
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    stopped(cancel)?;
    let FileOperation {
        space,
        session,
        by,
        generation,
    } = *op;
    let path = target_path(
        session,
        root,
        &change.path,
        change.previous.is_none().then_some(op),
    )
    .await?;
    verify_change(session, root, change).await?;
    let parent = path.rsplit_once('/').expect("absolute path").0;
    let staged = format!("{parent}/.vibe-upload-{}", token(16));
    let backup = change
        .previous
        .as_ref()
        .map(|_| format!("{parent}/.vibe-backup-{}", token(16)));
    let handle = space
        .file_write(
            by,
            generation,
            session.open(
                &staged,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                FileAttributes {
                    permissions: Some(0o600),
                    ..Default::default()
                },
            ),
        )
        .await?
        .handle;
    let mut publishing = false;
    let result = async {
        for (i, chunk) in data.chunks(MAX_INPUT).enumerate() {
            stopped(cancel)?;
            space
                .file_write(
                    by,
                    generation,
                    session.write(&handle, (i * MAX_INPUT) as u64, chunk.to_vec()),
                )
                .await?;
        }
        if let Some(old) = &change.previous {
            let attrs = &old.attrs;
            space
                .file_write(
                    by,
                    generation,
                    session.fsetstat(
                        &handle,
                        FileAttributes {
                            permissions: attrs.permissions.map(|mode| mode & 0o777),
                            uid: attrs.uid,
                            gid: attrs.gid,
                            ..Default::default()
                        },
                    ),
                )
                .await?;
        }
        space
            .file_write(by, generation, session.close(&handle))
            .await?;
        target_path(session, root, &change.path, None).await?;
        verify_change(session, root, change).await?;
        if let (Some(backup), Some(old)) = (&backup, &change.previous) {
            space
                .file_write(by, generation, session.hardlink(&path, backup))
                .await?;
            let saved = backup.rsplit_once('/').expect("absolute backup").1;
            let mut budget = FILE_BYTES;
            let valid = remote_digest(session, parent_or_root(parent), saved, &mut budget)
                .await?
                .is_some_and(|now| {
                    now.digest == old.digest && same_file_version(&now.attrs, &old.attrs)
                });
            if !valid {
                return Err(changed(&change.path));
            }
            verify_change(session, root, change).await?;
        }
        stopped(cancel)?;
        target_path(session, root, &change.path, None).await?;
        publishing = true;
        if backup.is_some() {
            space
                .file_write(by, generation, editor::replace(session, &staged, &path))
                .await?;
        } else {
            space
                .file_write(by, generation, session.rename(&staged, &path))
                .await?;
        }
        let mut budget = FILE_BYTES;
        if remote_digest(session, root, &change.path, &mut budget)
            .await?
            .is_none_or(|now| !now.digest.eq_ignore_ascii_case(&change.digest))
        {
            return err(
                ErrorCode::DeliveryUnknown,
                "发布后读回的内容已变化，请核实。",
            );
        }
        Ok(backup.clone())
    }
    .await;
    if result.is_err() {
        let _ = session.close(handle).await;
    }
    result.map_err(|mut error| {
        if publishing
            && !matches!(
                error.code,
                ErrorCode::ControlRevoked | ErrorCode::StaleLease
            )
        {
            error.code = ErrorCode::DeliveryUnknown;
        }
        error.message.push_str(&format!(" 临时路径：{staged}。"));
        if let Some(backup) = backup {
            error
                .message
                .push_str(&format!(" 备份路径（若已创建）：{backup}。"));
        }
        error
    })
}

fn parent_or_root(parent: &str) -> &str {
    if parent.is_empty() { "/" } else { parent }
}
