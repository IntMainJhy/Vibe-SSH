//! Read-only remote half of directory comparison; local files never enter this module.
use super::*;
use crate::diff::{self, DEPTH, ENTRIES, FILE_BYTES, HASH_BYTES, PATH_BYTES};
use sha2::{Digest, Sha256};
use vibe_ssh_protocol::{DiffEntry, DiffPreview, Manifest, ManifestEntry};

pub(super) struct RemoteVersion {
    pub(super) digest: String,
    pub(super) attrs: FileAttributes,
}

pub struct DiffApplyPlan {
    pub(super) space: String,
    pub(super) root: String,
    pub(super) generation: String,
    pub(super) files: Vec<DiffChange>,
}

pub(super) struct DiffChange {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) digest: String,
    pub(super) previous: Option<RemoteVersion>,
}

impl DiffApplyPlan {
    pub fn entries(&self) -> impl Iterator<Item = (&str, bool)> {
        self.files
            .iter()
            .map(|file| (file.path.as_str(), file.previous.is_some()))
    }
}

impl Service {
    pub async fn diff(
        &self,
        id: &str,
        remote: &str,
        manifest: Manifest,
        agent: bool,
    ) -> Result<DiffPreview> {
        Ok(self
            .compare_diff(id, remote, manifest, agent, false)
            .await?
            .0)
    }

    pub async fn review_diff(
        &self,
        id: &str,
        remote: &str,
        manifest: Manifest,
    ) -> Result<(DiffPreview, Option<DiffApplyPlan>)> {
        self.compare_diff(id, remote, manifest, false, true).await
    }

    async fn compare_diff(
        &self,
        id: &str,
        remote: &str,
        manifest: Manifest,
        agent: bool,
        review: bool,
    ) -> Result<(DiffPreview, Option<DiffApplyPlan>)> {
        path_valid(remote)?;
        diff::validate(&manifest)?;
        let rules = diff::rules(&manifest.excludes)?;
        let space = self.remote_read_space(id, agent)?;
        let generation = if review {
            space.generation(By::Human(id)).await.ok()
        } else {
            None
        };
        let _operation = space.file_operation.try_lock().map_err(|_| {
            Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作，请稍后比较。")
        })?;
        let sftp = space.sftp().await?;
        let root = sftp
            .realpath(remote)
            .await
            .map_err(sftp_error)?
            .files
            .into_iter()
            .next()
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "no canonical remote path"))?
            .filename;
        path_valid(&root)?;
        if !sftp
            .lstat(&root)
            .await
            .map_err(sftp_error)?
            .attrs
            .file_type()
            .is_dir()
        {
            return err(
                ErrorCode::InvalidRequest,
                "comparison root must be a directory",
            );
        }
        let mut entries = BTreeMap::<String, ManifestEntry>::new();
        let mut pending = vec![(String::new(), 0)];
        let mut bytes = 0;
        let mut truncated = manifest.truncated;
        'scan: while let Some((parent, depth)) = pending.pop() {
            let full = format!("{}/{}", root.trim_end_matches('/'), parent);
            if !sftp
                .lstat(&full)
                .await
                .map_err(sftp_error)?
                .attrs
                .file_type()
                .is_dir()
            {
                return err(
                    ErrorCode::FileChanged,
                    "directory changed during comparison",
                );
            }
            let handle = sftp.opendir(&full).await.map_err(sftp_error)?.handle;
            let batch_result = async {
                let mut stop = false;
                loop {
                    let batch = match sftp.readdir(&handle).await {
                        Ok(batch) => batch,
                        Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                        Err(e) => return Err(sftp_error(e)),
                    };
                    if batch.files.is_empty() {
                        return err(
                            ErrorCode::InvalidRequest,
                            "empty directory page without EOF",
                        );
                    }
                    for file in batch.files {
                        let name = file.filename;
                        if name == "." || name == ".." {
                            continue;
                        }
                        if name.is_empty() || name.contains(['/', '\0']) {
                            return err(
                                ErrorCode::InvalidRequest,
                                "invalid remote directory entry",
                            );
                        }
                        let path = if parent.is_empty() {
                            name
                        } else {
                            format!("{parent}/{name}")
                        };
                        if entries.len() == ENTRIES
                            || bytes + path.len() > PATH_BYTES
                            || depth >= DEPTH
                        {
                            stop = true;
                            break;
                        }
                        bytes += path.len();
                        let attrs = file.attrs;
                        let kind = if attrs.file_type().is_dir() {
                            "directory"
                        } else if attrs.file_type().is_file() {
                            "file"
                        } else if attrs.file_type().is_symlink() {
                            "symlink"
                        } else {
                            "other"
                        };
                        let excluded = diff::excluded(&path, &rules);
                        if kind == "directory" && !excluded {
                            pending.push((path.clone(), depth + 1));
                        }
                        entries.insert(
                            path.clone(),
                            ManifestEntry {
                                path,
                                kind: kind.into(),
                                size: attrs.size,
                                digest: None,
                                excluded,
                            },
                        );
                    }
                    if stop {
                        break;
                    }
                }
                Ok(stop)
            }
            .await;
            let closed = sftp.close(handle).await.map_err(sftp_error);
            let stop = batch_result?;
            closed?;
            if stop {
                truncated = true;
                break 'scan;
            }
        }
        let mut local: BTreeMap<_, _> = manifest
            .entries
            .into_iter()
            .map(|e| (e.path.clone(), e))
            .collect();
        let paths: std::collections::BTreeSet<_> =
            local.keys().chain(entries.keys()).cloned().collect();
        let mut budget = HASH_BYTES;
        let mut result = Vec::new();
        let mut changes = Vec::new();
        for path in paths {
            let left = local.remove(&path);
            let right = entries.remove(&path);
            let mut previous = None;
            let status =
                if diff::excluded(&path, &rules) || left.as_ref().is_some_and(|e| e.excluded) {
                    "excluded"
                } else {
                    match (&left, &right) {
                        (Some(l), Some(r)) if l.kind != r.kind => "type_conflict",
                        (Some(l), Some(_)) if l.kind == "directory" => "directory",
                        (Some(l), Some(_)) if l.kind != "file" => "unsupported",
                        (Some(l), Some(r))
                            if l.digest.is_some()
                                && r.size.is_some_and(|s| s <= FILE_BYTES && s <= budget) =>
                        {
                            match remote_digest(&sftp, &root, &path, &mut budget).await? {
                                Some(version)
                                    if l.digest.as_ref().is_some_and(|d| {
                                        d.eq_ignore_ascii_case(&version.digest)
                                    }) =>
                                {
                                    "same"
                                }
                                Some(version) => {
                                    previous = Some(version);
                                    "changed"
                                }
                                None => "unverified",
                            }
                        }
                        (Some(_), Some(_)) => "unverified",
                        (Some(_), None) if !truncated => "local_only",
                        (None, Some(_)) if !truncated => "remote_only",
                        _ => "unverified",
                    }
                };
            if generation.is_some()
                && !truncated
                && (status == "local_only" || (status == "changed" && sftp.editable))
                && let Some(entry) = &left
                && let (Some(size), Some(digest)) = (entry.size, &entry.digest)
            {
                changes.push(DiffChange {
                    path: path.clone(),
                    size,
                    digest: digest.clone(),
                    previous,
                });
            }
            result.push(DiffEntry {
                path,
                local_size: left.and_then(|e| e.size),
                remote_size: right.and_then(|e| e.size),
                status: status.into(),
            });
        }
        let plan = generation
            .filter(|_| root.starts_with('/') && !changes.is_empty())
            .map(|generation| DiffApplyPlan {
                space: id.into(),
                root: root.clone(),
                generation,
                files: changes,
            });
        Ok((
            DiffPreview {
                remote: root,
                entries: result,
                truncated,
            },
            plan,
        ))
    }
}
pub(super) async fn remote_digest(
    sftp: &FileSession,
    root: &str,
    path: &str,
    budget: &mut u64,
) -> Result<Option<RemoteVersion>> {
    let mut full = root.trim_end_matches('/').to_owned();
    let parts: Vec<_> = path.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        full.push('/');
        full.push_str(part);
        if !sftp
            .lstat(&full)
            .await
            .map_err(sftp_error)?
            .attrs
            .file_type()
            .is_dir()
        {
            return err(
                ErrorCode::FileChanged,
                "remote ancestor changed or became a link",
            );
        }
    }
    full.push('/');
    full.push_str(parts[parts.len() - 1]);
    let before = sftp.lstat(&full).await.map_err(sftp_error)?.attrs;
    if !before.file_type().is_file() {
        return err(
            ErrorCode::FileChanged,
            "selected remote file became a link or special file",
        );
    }
    if before.size.is_none_or(|s| s > FILE_BYTES || s > *budget) {
        return Ok(None);
    }
    let handle = sftp
        .open(&full, OpenFlags::READ, FileAttributes::default())
        .await
        .map_err(sftp_error)?
        .handle;
    let result = async {
        let opened = sftp.fstat(&handle).await.map_err(sftp_error)?.attrs;
        if !same_file_version(&before, &opened) {
            return err(ErrorCode::FileChanged, "remote file changed while opening");
        }
        let mut hash = Sha256::new();
        let mut count = 0;
        let expected = opened.size.unwrap_or_default();
        while count < expected {
            let data = match sftp
                .read(
                    &handle,
                    count,
                    (expected - count).min(MAX_INPUT as u64) as u32,
                )
                .await
            {
                Ok(data) => data.data,
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(sftp_error(e)),
            };
            if data.is_empty() {
                return err(ErrorCode::ConnectionFailed, "empty SFTP data without EOF");
            }
            count += data.len() as u64;
            if count > expected || count > *budget {
                return err(
                    ErrorCode::FileChanged,
                    "remote file grew beyond its comparison budget",
                );
            }
            hash.update(data);
        }
        if !same_file_version(
            &opened,
            &sftp.fstat(&handle).await.map_err(sftp_error)?.attrs,
        ) || !same_file_version(&opened, &sftp.lstat(&full).await.map_err(sftp_error)?.attrs)
            || opened.size != Some(count)
        {
            return err(ErrorCode::FileChanged, "remote file changed while reading");
        }
        *budget -= count;
        Ok(Some(RemoteVersion {
            digest: format!("{:x}", hash.finalize()),
            attrs: opened,
        }))
    }
    .await;
    let closed = sftp.close(handle).await.map_err(sftp_error);
    let digest = result?;
    closed?;
    Ok(digest)
}
