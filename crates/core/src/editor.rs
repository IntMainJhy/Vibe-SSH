//! Bounded remote text reads and desktop-only optimistic save.
use super::*;
use russh_sftp::protocol::Packet;
use sha2::{Digest, Sha256};

const TEXT_LIMIT: usize = 1_048_576;
const DOCUMENTS: usize = 8;

#[derive(Debug, Serialize)]
pub struct TextDocument {
    pub path: String,
    pub text: String,
    pub revision: String,
    pub save_supported: bool,
}
#[derive(Debug, Serialize)]
pub struct TextSaved {
    pub document: TextDocument,
    pub backup: Option<String>,
}
#[derive(Clone)]
pub(in crate::service) struct EditBase {
    path: String,
    fingerprint: String,
    generation: String,
}
struct Contents {
    bytes: Vec<u8>,
    attrs: FileAttributes,
}
impl Contents {
    fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(&self.bytes);
        let attrs = &self.attrs;
        hash.update(
            serde_json::to_vec(&(
                attrs.size,
                attrs.mtime,
                attrs.permissions,
                attrs.uid,
                attrs.gid,
            ))
            .expect("plain metadata"),
        );
        format!("{:x}", hash.finalize())
    }
}
fn conflict(message: &str) -> Error {
    Error::new(ErrorCode::FileChanged, message)
}
fn metadata_equal(a: &FileAttributes, b: &FileAttributes) -> bool {
    (a.size, a.mtime, a.permissions, a.uid, a.gid) == (b.size, b.mtime, b.permissions, b.uid, b.gid)
}
async fn read(session: &FileSession, path: &str, limit: usize) -> Result<Contents> {
    let before = session.lstat(path).await.map_err(sftp_error)?.attrs;
    if !before.file_type().is_file() {
        return err(
            ErrorCode::InvalidRequest,
            "只能预览普通文本文件；不跟随符号链接或读取特殊文件。",
        );
    }
    if before.size.is_none_or(|size| size > limit as u64) {
        return err(
            ErrorCode::InvalidRequest,
            format!("无法确认文件在 {limit} 字节读取上限内。"),
        );
    }
    let handle = session
        .open(path, OpenFlags::READ, FileAttributes::default())
        .await
        .map_err(sftp_error)?
        .handle;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let opened = session.fstat(&handle).await.map_err(sftp_error)?.attrs;
        if !opened.file_type().is_file() || !metadata_equal(&before, &opened) {
            return Err(conflict("读取期间文件已变化，请重新打开。"));
        }
        let mut bytes = Vec::with_capacity(opened.size.unwrap_or_default() as usize);
        loop {
            match session
                .read(
                    &handle,
                    bytes.len() as u64,
                    (limit - bytes.len() + 1).min(MAX_INPUT) as u32,
                )
                .await
            {
                Ok(data) => {
                    if data.data.is_empty() {
                        return err(ErrorCode::ConnectionFailed, "empty SFTP data without EOF");
                    }
                    if bytes.len() + data.data.len() > limit {
                        return err(
                            ErrorCode::ResourceLimit,
                            "text grew beyond the byte limit during reading",
                        );
                    }
                    bytes.extend_from_slice(&data.data);
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(error) => return Err(sftp_error(error)),
            }
        }
        let after = session.fstat(&handle).await.map_err(sftp_error)?.attrs;
        let named = session.lstat(path).await.map_err(sftp_error)?.attrs;
        if !named.file_type().is_file()
            || !metadata_equal(&opened, &after)
            || !metadata_equal(&after, &named)
            || after.size != Some(bytes.len() as u64)
        {
            return Err(conflict("读取期间文件已变化，请重新打开。"));
        }
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return err(
                ErrorCode::InvalidRequest,
                "此文件不是 UTF-8 文本，请下载后使用对应工具打开。",
            );
        }
        Ok(Contents {
            bytes,
            attrs: after,
        })
    })
    .await;
    let closed = session.close(handle).await.map_err(sftp_error);
    let contents =
        result.map_err(|_| Error::new(ErrorCode::ConnectionFailed, "text read timed out"))??;
    closed?;
    Ok(contents)
}
fn document(
    space: &Space,
    path: String,
    contents: Contents,
    generation: String,
    supported: bool,
    replacing: Option<&str>,
) -> TextDocument {
    let revision = token(32);
    let mut versions = space.documents.lock().unwrap();
    if let Some(revision) = replacing {
        versions.retain(|(id, _)| id != revision);
    }
    // Keep different editor reads independent, including two windows reading the same path.
    versions.push_back((
        revision.clone(),
        EditBase {
            path: path.clone(),
            fingerprint: contents.fingerprint(),
            generation,
        },
    ));
    while versions.len() > DOCUMENTS {
        versions.pop_front();
    }
    TextDocument {
        path,
        text: String::from_utf8(contents.bytes).expect("validated UTF-8"),
        revision,
        save_supported: supported,
    }
}
fn rename_payload(from: &str, to: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(from.len() + to.len() + 8);
    for path in [from, to] {
        bytes.extend_from_slice(&(path.len() as u32).to_be_bytes());
        bytes.extend_from_slice(path.as_bytes());
    }
    bytes
}
pub(super) async fn replace(
    session: &FileSession,
    from: &str,
    to: &str,
) -> std::result::Result<(), SftpError> {
    match session
        .extended("posix-rename@openssh.com", rename_payload(from, to))
        .await?
    {
        Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
        Packet::Status(status) => Err(SftpError::Status(status)),
        _ => Err(SftpError::UnexpectedPacket),
    }
}

impl Service {
    pub(crate) async fn agent_text(
        &self,
        space: &str,
        path: &str,
        max_bytes: usize,
    ) -> Result<Value> {
        path_valid(path)?;
        if !(1..=vibe_ssh_protocol::MAX_TEXT_BYTES).contains(&max_bytes) {
            return err(
                ErrorCode::InvalidRequest,
                "text limit must be 1..=65536 bytes",
            );
        }
        let space = self.remote_read_space(space, true)?;
        let _file = space.file_operation.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "another file operation is in progress in this space",
            )
        })?;
        let session = space.sftp().await?;
        let contents = read(&session, path, max_bytes).await?;
        let bytes = contents.bytes.len();
        Ok(
            json!({"path": path, "bytes": bytes, "text": String::from_utf8(contents.bytes).expect("validated UTF-8")}),
        )
    }

    pub async fn text_file(&self, space: &str, path: &str) -> Result<TextDocument> {
        path_valid(path)?;
        let space = self.find_space(space, false)?;
        let _file = space.file_operation.try_lock().map_err(|_| {
            Error::new(
                ErrorCode::ResourceLimit,
                "该空间已有文件操作，请稍后再打开。",
            )
        })?;
        let generation = space.control.lock().await.lease.clone();
        let session = space.sftp().await?;
        // Resolve parent links only after refusing a symlink at the selected file itself.
        if !session
            .lstat(path)
            .await
            .map_err(sftp_error)?
            .attrs
            .file_type()
            .is_file()
        {
            return err(ErrorCode::InvalidRequest, "请选择普通文本文件。");
        }
        let canonical = session
            .realpath(path)
            .await
            .map_err(sftp_error)?
            .files
            .into_iter()
            .next()
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "server returned no path"))?
            .filename;
        path_valid(&canonical)?;
        let contents = read(&session, &canonical, TEXT_LIMIT).await?;
        let document = document(
            &space,
            canonical,
            contents,
            generation,
            session.editable,
            None,
        );
        bump(&self.events);
        Ok(document)
    }
    pub fn forget_text(&self, space: &str, revision: &str) -> Result<()> {
        self.find_space(space, false)?
            .documents
            .lock()
            .unwrap()
            .retain(|(id, _)| id != revision);
        bump(&self.events);
        Ok(())
    }
    pub async fn save_text(&self, by: By<'_>, revision: &str, text: &str) -> Result<TextSaved> {
        if text.len() > TEXT_LIMIT || text.contains('\0') {
            return err(
                ErrorCode::InvalidRequest,
                "只可保存不超过 1 MiB 的 UTF-8 文本。",
            );
        }
        let space = self.space_of(&by)?;
        let generation = space.generation(by).await?;
        let base = space
            .documents
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == revision)
            .map(|(_, base)| base.clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::NotFound,
                    "文件版本已过期，请保留草稿并重新打开文件。",
                )
            })?;
        if base.generation != generation {
            return err(
                ErrorCode::StaleLease,
                "控制权已变化，请保留草稿并重新读取远端版本。",
            );
        }
        let _save = self
            .editor_save
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "已有文本正在保存，请稍后再试。"))?;
        let _file = space.file_operation.try_lock().map_err(|_| {
            Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作，请稍后保存。")
        })?;
        let session = space.sftp().await?;
        if !session.editable {
            return err(
                ErrorCode::InvalidRequest,
                "服务器不支持安全替换与备份，此处只能预览；可使用终端编辑。",
            );
        }
        let original = read(&session, &base.path, TEXT_LIMIT).await?;
        if original.fingerprint() != base.fingerprint {
            return Err(conflict(
                "远端文件已被修改，未覆盖任何内容。草稿已保留，请复制草稿或重新读取远端版本。",
            ));
        }
        if original.bytes == text.as_bytes() {
            return Ok(TextSaved {
                document: document(
                    &space,
                    base.path,
                    original,
                    generation,
                    session.editable,
                    Some(revision),
                ),
                backup: None,
            });
        }
        let parent = base.path.rsplit_once('/').map_or("", |(parent, _)| parent);
        let temporary = format!("{parent}/.vibe-edit-{}", token(16));
        let backup = format!("{parent}/.vibe-backup-{}", token(16));
        let handle = space
            .file_write(
                by,
                &generation,
                session.open(
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
        let mut committing = false;
        let result = async {
            for (chunk, data) in text.as_bytes().chunks(MAX_INPUT).enumerate() {
                space
                    .file_write(
                        by,
                        &generation,
                        session.write(&handle, (chunk * MAX_INPUT) as u64, data.to_vec()),
                    )
                    .await?;
            }
            // Refuse the save if the account cannot preserve the original owner/mode.
            let attrs = FileAttributes {
                permissions: original.attrs.permissions.map(|mode| mode & 0o7777),
                uid: original.attrs.uid,
                gid: original.attrs.gid,
                ..Default::default()
            };
            space
                .file_write(by, &generation, session.fsetstat(&handle, attrs))
                .await?;
            space
                .file_write(by, &generation, session.close(&handle))
                .await?;
            if read(&session, &base.path, TEXT_LIMIT).await?.fingerprint() != base.fingerprint {
                return Err(conflict("上传期间远端文件已变化，未替换目标。草稿已保留。"));
            }
            space
                .file_write(
                    by,
                    &generation,
                    session.hardlink(base.path.clone(), backup.clone()),
                )
                .await?;
            if read(&session, &backup, TEXT_LIMIT).await?.fingerprint() != base.fingerprint
                || read(&session, &base.path, TEXT_LIMIT).await?.fingerprint() != base.fingerprint
            {
                return Err(conflict("保存前远端版本已变化，未替换目标。草稿已保留。"));
            }
            committing = true;
            space
                .file_write(by, &generation, replace(&session, &temporary, &base.path))
                .await?;
            let current = read(&session, &base.path, TEXT_LIMIT).await?;
            if current.bytes != text.as_bytes() {
                return Err(conflict(
                    "保存后远端内容又被修改，请核实结果；不会自动重试。",
                ));
            }
            Ok(TextSaved {
                document: document(
                    &space,
                    base.path.clone(),
                    current,
                    generation,
                    session.editable,
                    Some(revision),
                ),
                backup: Some(backup.clone()),
            })
        }
        .await;
        if result.is_err() {
            let _ = session.close(handle).await;
        }
        result.map_err(|mut error| {
            if committing
                && !matches!(
                    error.code,
                    ErrorCode::ControlRevoked | ErrorCode::StaleLease | ErrorCode::FileChanged
                )
            {
                error.code = ErrorCode::DeliveryUnknown;
                error.message = format!(
                    "保存结果需要核实，可能已写入服务器；请保留草稿后重新读取，不要重复提交。{}",
                    error.message
                );
            }
            error.message = format!(
                "{} 临时文件：{}；备份（若已创建）：{}。",
                error.message, temporary, backup
            );
            error
        })
    }
}
