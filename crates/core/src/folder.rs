use super::resume_upload::{parent, unused_destination, verify_parent};
use super::*;
use rustix::fs::{Mode, OFlags, RenameFlags, open, renameat_with};
use std::{
    fs::Metadata,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tokio::fs::{self, File, OpenOptions};

const MAX_DEPTH: usize = 16;
const MAX_ENTRIES: usize = 5000;
const MAX_PATH_BYTES: usize = 512 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Node {
    size: Option<u64>,
    version: String,
    mode: u32,
}

type Tree = BTreeMap<String, Node>;

#[derive(Serialize, Deserialize)]
pub struct FolderPlan {
    pub host_id: String,
    pub upload: bool,
    pub remote: String,
    endpoint: (String, u16, String, String),
    local: PathBuf,
    local_parent: (u64, u64),
    token: String,
    source: Tree,
    #[serde(skip)]
    generation: Option<String>,
}

impl FolderPlan {
    pub fn name(&self) -> &str {
        self.remote.rsplit('/').next().unwrap_or_default()
    }

    pub fn total(&self) -> u64 {
        self.source
            .values()
            .filter_map(|node| node.size)
            .fold(0, u64::saturating_add)
    }

    pub fn staging(&self) -> Result<String> {
        if self.upload {
            child(parent(&self.remote)?, &self.stage_name()?)
        } else {
            Ok(self
                .local
                .with_file_name(self.stage_name()?)
                .to_string_lossy()
                .into_owned())
        }
    }

    fn stage_name(&self) -> Result<String> {
        if self.token.len() != 32 || !self.token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return err(ErrorCode::InvalidRequest, "文件夹续传记录无效。");
        }
        Ok(format!(".vibe-folder-{}", self.token))
    }
}

fn identity(host: &Host) -> (String, u16, String, String) {
    (
        host.host.clone(),
        host.port,
        host.username.clone(),
        host.host_key_sha256.clone(),
    )
}

fn leaf(name: &str) -> Result<&str> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\0'])
        || name.len() > 255
    {
        return err(ErrorCode::InvalidRequest, "目录包含无法传输的文件名。");
    }
    Ok(name)
}

fn child(path: &str, name: &str) -> Result<String> {
    at(path, leaf(name)?)
}

fn at(root: &str, relative: &str) -> Result<String> {
    for part in relative.split('/') {
        leaf(part)?;
    }
    let path = format!("{}/{}", root.trim_end_matches('/'), relative);
    path_valid(&path)?;
    Ok(path)
}

fn changed() -> Error {
    Error::new(
        ErrorCode::FileChanged,
        "源目录或临时文件已变化，请核实后重新传输。",
    )
}

fn add(tree: &mut Tree, path: String, node: Node, budget: &mut usize) -> Result<()> {
    *budget = budget.saturating_add(path.len());
    if tree.len() > MAX_ENTRIES || *budget > MAX_PATH_BYTES || path.split('/').count() > MAX_DEPTH {
        return err(
            ErrorCode::ResourceLimit,
            "文件夹最多传输 5000 项、16 层，相对路径合计最多 512 KiB。",
        );
    }
    if tree.insert(path, node).is_some() {
        return err(ErrorCode::FileChanged, "目录出现重复路径。");
    }
    Ok(())
}

fn relative(base: &str, name: &str) -> Result<String> {
    let name = leaf(name)?;
    Ok(if base.is_empty() {
        name.into()
    } else {
        format!("{base}/{name}")
    })
}

fn local_node(metadata: &Metadata) -> Result<Node> {
    let size = if metadata.is_file() {
        Some(metadata.len())
    } else if metadata.is_dir() {
        None
    } else {
        return err(ErrorCode::InvalidRequest, "文件夹包含符号链接或特殊文件。");
    };
    Ok(Node {
        size,
        version: format!(
            "{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ),
        mode: metadata.mode() & 0o7777,
    })
}

fn remote_node(attrs: &FileAttributes) -> Result<Node> {
    let size = if attrs.file_type().is_file() {
        Some(
            attrs
                .size
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "服务器没有提供文件大小。"))?,
        )
    } else if attrs.file_type().is_dir() {
        None
    } else {
        return err(ErrorCode::InvalidRequest, "文件夹包含符号链接或特殊文件。");
    };
    Ok(Node {
        size,
        version: format!("{:?}:{:?}:{:?}", attrs.mtime, attrs.uid, attrs.gid),
        mode: attrs.permissions.unwrap_or_default() & 0o7777,
    })
}

async fn local_tree(root: &Path, cancel: &AtomicBool) -> Result<Tree> {
    let mut tree = Tree::new();
    let mut budget = 0;
    let mut pending = vec![(String::new(), root.to_owned())];
    while let Some((key, path)) = pending.pop() {
        stopped(cancel)?;
        let node = local_node(&fs::symlink_metadata(&path).await?)?;
        let is_dir = node.size.is_none();
        if key.is_empty() && !is_dir {
            return Err(changed());
        }
        add(&mut tree, key.clone(), node, &mut budget)?;
        if is_dir {
            let mut entries = fs::read_dir(&path).await?;
            while let Some(entry) = entries.next_entry().await? {
                stopped(cancel)?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "文件名不是 UTF-8。"))?;
                if pending.len() + tree.len() > MAX_ENTRIES {
                    return err(ErrorCode::ResourceLimit, "文件夹最多传输 5000 项。");
                }
                pending.push((relative(&key, name)?, entry.path()));
            }
        }
    }
    Ok(tree)
}

async fn remote_tree(session: &FileSession, root: &str, cancel: &AtomicBool) -> Result<Tree> {
    let root_meta = session.lstat(root).await.map_err(sftp_error)?.attrs;
    if !root_meta.file_type().is_dir() {
        return err(ErrorCode::InvalidRequest, "请选择普通目录。");
    }
    verify_parent(session, root).await?;
    let mut tree = Tree::from([(String::new(), remote_node(&root_meta)?)]);
    let mut pending = vec![(String::new(), root.to_owned())];
    let mut budget = 0;
    while let Some((key, path)) = pending.pop() {
        stopped(cancel)?;
        verify_parent(session, &path).await?;
        let handle = session.opendir(&path).await.map_err(sftp_error)?.handle;
        let result = async {
            loop {
                stopped(cancel)?;
                let page = match session.readdir(&handle).await {
                    Ok(page) if !page.files.is_empty() => page,
                    Ok(_) => return err(ErrorCode::ConnectionFailed, "服务器返回空目录页。"),
                    Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                        break;
                    }
                    Err(error) => return Err(sftp_error(error)),
                };
                for item in page.files {
                    if matches!(item.filename.as_str(), "." | "..") {
                        continue;
                    }
                    let key = relative(&key, &item.filename)?;
                    let node = remote_node(&item.attrs)?;
                    if node.size.is_none() {
                        pending.push((key.clone(), child(&path, &item.filename)?));
                    }
                    add(&mut tree, key, node, &mut budget)?;
                }
            }
            Ok(())
        }
        .await;
        let _ = session.close(handle).await;
        result?;
    }
    Ok(tree)
}

fn verified_stage(source: &Tree, staged: &Tree) -> Result<()> {
    for (path, node) in staged {
        let target = source.get(path).ok_or_else(changed)?;
        let valid = match (node.size, target.size) {
            (None, None) => node.mode == 0o700,
            (Some(part), Some(size)) => node.mode == 0o600 && part <= size,
            _ => false,
        };
        if !valid {
            return Err(changed());
        }
    }
    Ok(())
}

fn dir_options() -> fs::DirBuilder {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
}

async fn local_parent(path: &Path) -> Result<(u64, u64)> {
    let directory = path.parent().ok_or_else(changed)?;
    let metadata = fs::symlink_metadata(directory).await?;
    if !metadata.is_dir() || fs::canonicalize(directory).await? != directory {
        return Err(changed());
    }
    Ok((metadata.dev(), metadata.ino()))
}

async fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => err(ErrorCode::FileChanged, "目标已存在，请核实最终目录。"),
    }
}

async fn local_file(path: &Path, create: bool, writing: bool) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(writing)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .await?)
}

impl Service {
    pub async fn plan_folder(
        &self,
        id: &str,
        folder: &Path,
        remote: &str,
        upload: bool,
        cancel: &AtomicBool,
    ) -> Result<FolderPlan> {
        path_valid(remote)?;
        if !remote.starts_with('/') {
            return err(ErrorCode::InvalidRequest, "请选择绝对远端路径。");
        }
        let space = self.find_space(id, false)?;
        let generation = if upload {
            Some(space.generation(By::Human(id)).await?)
        } else {
            None
        };
        let session = space.sftp().await?;
        let canonical = session
            .realpath(remote)
            .await
            .map_err(sftp_error)?
            .files
            .into_iter()
            .next()
            .ok_or_else(changed)?
            .filename;
        let (local, remote, source) = if upload {
            if !fs::symlink_metadata(folder).await?.is_dir() {
                return err(ErrorCode::InvalidRequest, "请选择普通文件夹。");
            }
            let name = folder
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(changed)?;
            let target = child(&canonical, name)?;
            unused_destination(&session, &target).await?;
            let folder = fs::canonicalize(folder).await?;
            let tree = local_tree(&folder, cancel).await?;
            (folder, target, tree)
        } else {
            if !session
                .lstat(remote.trim_end_matches('/'))
                .await
                .map_err(sftp_error)?
                .attrs
                .file_type()
                .is_dir()
            {
                return err(ErrorCode::InvalidRequest, "请选择普通文件夹。");
            }
            let name = leaf(canonical.rsplit('/').next().unwrap_or_default())?;
            let target = fs::canonicalize(folder).await?.join(name);
            absent(&target).await?;
            let tree = remote_tree(&session, &canonical, cancel).await?;
            (target, canonical, tree)
        };
        Ok(FolderPlan {
            host_id: space.host.id.clone(),
            endpoint: identity(&space.host),
            local_parent: local_parent(&local).await?,
            local,
            remote,
            upload,
            source,
            token: token(32),
            generation,
        })
    }

    pub async fn transfer_folder_plan(
        &self,
        id: &str,
        plan: &FolderPlan,
        resume: bool,
        cancel: &AtomicBool,
        progress: impl Fn(u64),
    ) -> Result<()> {
        let by = By::Human(id);
        let space = self.find_space(id, false)?;
        if plan.host_id != space.host.id || plan.endpoint != identity(&space.host) {
            return err(ErrorCode::InvalidRequest, "请连接原服务器后继续。");
        }
        let generation = if plan.upload {
            let current = space.generation(by).await?;
            if !resume && plan.generation.as_ref() != Some(&current) {
                return err(ErrorCode::StaleLease, "上传期间控制权已变化。");
            }
            current
        } else {
            String::new()
        };
        let _guard = space
            .file_operation
            .try_lock()
            .map_err(|_| Error::new(ErrorCode::ResourceLimit, "该空间已有文件操作。"))?;
        stopped(cancel)?;
        if plan.local_parent != local_parent(&plan.local).await? {
            return Err(changed());
        }
        let session = space.sftp().await?;
        let operation = FileOperation {
            space: &space,
            session: &session,
            by,
            generation: &generation,
        };
        let transfer = FolderTransfer {
            operation,
            plan,
            cancel,
            progress,
        };
        let result = if plan.upload {
            transfer.upload(resume).await
        } else {
            transfer.download(resume).await
        };
        let staging = plan.staging()?;
        result.map_err(|mut error| {
            error.message.push_str(&format!(" 临时目录 {staging}。"));
            error
        })
    }

    pub async fn upload_folder(
        &self,
        id: &str,
        source: &Path,
        destination: &str,
        cancel: &AtomicBool,
        progress: impl Fn(u64),
    ) -> Result<()> {
        let plan = self
            .plan_folder(id, source, destination, true, cancel)
            .await?;
        self.transfer_folder_plan(id, &plan, false, cancel, progress)
            .await
    }

    pub async fn download_folder(
        &self,
        id: &str,
        remote: &str,
        parent: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(u64),
    ) -> Result<()> {
        let plan = self.plan_folder(id, parent, remote, false, cancel).await?;
        self.transfer_folder_plan(id, &plan, false, cancel, progress)
            .await
    }
}

struct FolderTransfer<'a, P> {
    operation: FileOperation<'a>,
    plan: &'a FolderPlan,
    cancel: &'a AtomicBool,
    progress: P,
}

impl<P: Fn(u64)> FolderTransfer<'_, P> {
    async fn upload(self, resume: bool) -> Result<()> {
        let Self {
            operation: op,
            plan,
            cancel,
            progress,
        } = self;
        let session = op.session;
        let staging = plan.staging()?;
        verify_parent(session, parent(&plan.remote)?).await?;
        unused_destination(session, &plan.remote).await?;
        if local_tree(&plan.local, cancel).await? != plan.source {
            return Err(changed());
        }
        if !resume {
            create_remote_dir(&op, &staging).await?;
        }
        let staged = remote_tree(session, &staging, cancel).await?;
        verified_stage(&plan.source, &staged)?;
        let mut verified = 0_u64;
        for (path, original) in plan.source.iter().filter(|(path, _)| !path.is_empty()) {
            stopped(cancel)?;
            let target = at(&staging, path)?;
            verify_parent(session, parent(&target)?).await?;
            let prior = staged.get(path);
            if let Some(size) = original.size {
                let source = plan.local.join(path);
                check_local_ancestors(&plan.local, &source).await?;
                let mut input = local_file(&source, false, false).await?;
                if local_node(&input.metadata().await?)? != *original {
                    return Err(changed());
                }
                put_file(&op, &target, prior, &mut input, size, cancel, |done| {
                    progress(verified.saturating_add(done))
                })
                .await?;
                if local_node(&input.metadata().await?)? != *original {
                    return Err(changed());
                }
                verified = verified.saturating_add(size);
            } else if prior.is_none() {
                create_remote_dir(&op, &target).await?;
            }
        }
        if local_tree(&plan.local, cancel).await? != plan.source {
            return Err(changed());
        }
        complete_tree(&plan.source, &remote_tree(session, &staging, cancel).await?)?;
        stopped(cancel)?;
        verify_parent(session, parent(&plan.remote)?).await?;
        op.space
            .file_write(op.by, op.generation, session.rename(&staging, &plan.remote))
            .await?;
        progress(verified);
        Ok(())
    }

    async fn download(self, resume: bool) -> Result<()> {
        let Self {
            operation: op,
            plan,
            cancel,
            progress,
        } = self;
        let session = op.session;
        let staging = plan.local.with_file_name(plan.stage_name()?);
        absent(&plan.local).await?;
        if remote_tree(session, &plan.remote, cancel).await? != plan.source {
            return Err(changed());
        }
        if !resume {
            dir_options().create(&staging).await?;
        }
        let staged = local_tree(&staging, cancel).await?;
        verified_stage(&plan.source, &staged)?;
        let mut verified = 0_u64;
        for (path, original) in plan.source.iter().filter(|(path, _)| !path.is_empty()) {
            stopped(cancel)?;
            let target = staging.join(path);
            check_local_ancestors(&staging, &target).await?;
            let prior = staged.get(path);
            if let Some(size) = original.size {
                let remote = at(&plan.remote, path)?;
                verify_parent(session, parent(&remote)?).await?;
                let mut output = local_file(&target, prior.is_none(), true).await?;
                let opened = output.metadata().await?;
                if !opened.is_file()
                    || opened.nlink() != 1
                    || opened.mode() & 0o7777 != 0o600
                    || prior.is_some_and(|node| local_node(&opened).ok().as_ref() != Some(node))
                {
                    return Err(changed());
                }
                get_file(session, &remote, original, &mut output, cancel, |done| {
                    progress(verified.saturating_add(done))
                })
                .await?;
                output.sync_all().await?;
                verified = verified.saturating_add(size);
            } else if prior.is_none() {
                dir_options().create(&target).await?;
            }
        }
        if remote_tree(session, &plan.remote, cancel).await? != plan.source {
            return Err(changed());
        }
        complete_tree(&plan.source, &local_tree(&staging, cancel).await?)?;
        stopped(cancel)?;
        publish_local(plan, &staging)?;
        progress(verified);
        Ok(())
    }
}

async fn check_local_ancestors(root: &Path, file: &Path) -> Result<()> {
    let parent = file.parent().ok_or_else(changed)?;
    if !parent.starts_with(root) || fs::canonicalize(parent).await? != parent {
        return Err(changed());
    }
    Ok(())
}

fn complete_tree(source: &Tree, staged: &Tree) -> Result<()> {
    verified_stage(source, staged)?;
    if source.len() != staged.len()
        || source
            .iter()
            .any(|(path, node)| staged[path].size != node.size)
    {
        return Err(changed());
    }
    Ok(())
}

fn publish_local(plan: &FolderPlan, staging: &Path) -> Result<()> {
    let parent = plan.local.parent().ok_or_else(changed)?;
    let directory = std::fs::File::from(
        open(
            parent,
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    let meta = directory.metadata()?;
    if (meta.dev(), meta.ino()) != plan.local_parent {
        return Err(changed());
    }
    let from = staging.file_name().ok_or_else(changed)?;
    let to = plan.local.file_name().ok_or_else(changed)?;
    renameat_with(&directory, from, &directory, to, RenameFlags::NOREPLACE).map_err(|error| {
        if error == rustix::io::Errno::EXIST {
            changed()
        } else {
            std::io::Error::from(error).into()
        }
    })
}

async fn create_remote_dir(op: &FileOperation<'_>, path: &str) -> Result<()> {
    op.space
        .file_write(
            op.by,
            op.generation,
            op.session.mkdir(
                path,
                FileAttributes {
                    permissions: Some(0o700),
                    ..Default::default()
                },
            ),
        )
        .await?;
    Ok(())
}

async fn compare_prefix(
    session: &FileSession,
    handle: &str,
    local: &mut File,
    offset: u64,
    buffer: &mut [u8],
    cancel: &AtomicBool,
) -> Result<()> {
    let mut done = 0;
    while done < offset {
        stopped(cancel)?;
        let count = (offset - done).min(buffer.len() as u64) as u32;
        let data = read_download(session, handle, done, count)
            .await?
            .ok_or_else(changed)?;
        local.read_exact(&mut buffer[..data.len()]).await?;
        if data != buffer[..data.len()] {
            return Err(changed());
        }
        done += data.len() as u64;
    }
    Ok(())
}

async fn put_file(
    op: &FileOperation<'_>,
    target: &str,
    prior: Option<&Node>,
    local: &mut File,
    size: u64,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<()> {
    let session = op.session;
    let mut flags = OpenFlags::READ | OpenFlags::WRITE;
    if let Some(prior) = prior {
        if remote_node(&regular_file(session, target).await?)? != *prior {
            return Err(changed());
        }
    } else {
        flags |= OpenFlags::CREATE | OpenFlags::EXCLUDE;
    }
    let handle = op
        .space
        .file_write(
            op.by,
            op.generation,
            session.open(
                target,
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
        let before = remote_node(&session.fstat(&handle).await.map_err(sftp_error)?.attrs)?;
        let mut done = prior.and_then(|node| node.size).unwrap_or(0);
        if before.mode != 0o600
            || before.size != Some(done)
            || prior.is_some_and(|node| node != &before)
            || done > size
        {
            return Err(changed());
        }
        let mut buffer = vec![0; MAX_INPUT];
        compare_prefix(session, &handle, local, done, &mut buffer, cancel).await?;
        progress(done);
        while done < size {
            stopped(cancel)?;
            let n = (size - done).min(buffer.len() as u64) as usize;
            local.read_exact(&mut buffer[..n]).await?;
            op.space
                .file_write(
                    op.by,
                    op.generation,
                    session.write(&handle, done, buffer[..n].to_vec()),
                )
                .await?;
            done += n as u64;
            progress(done);
        }
        let actual = session.fstat(&handle).await.map_err(sftp_error)?.attrs;
        if actual.size != Some(size)
            || !same_file_version(&actual, &regular_file(session, target).await?)
        {
            return Err(changed());
        }
        op.space
            .file_write(op.by, op.generation, session.close(&handle))
            .await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = session.close(handle).await;
    }
    result
}

async fn get_file(
    session: &FileSession,
    remote: &str,
    original: &Node,
    local: &mut File,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<()> {
    let handle = open_download(session, remote).await?;
    let result = async {
        let before = session.fstat(&handle).await.map_err(sftp_error)?.attrs;
        let offset = local.metadata().await?.len();
        if remote_node(&before)? != *original || before.size.is_none_or(|size| offset > size) {
            return Err(changed());
        }
        compare_prefix(
            session,
            &handle,
            local,
            offset,
            &mut vec![0; MAX_INPUT],
            cancel,
        )
        .await?;
        progress(offset);
        write_download(session, &handle, before, offset, local, cancel, |p| {
            progress(p.done)
        })
        .await?;
        if remote_node(&regular_file(session, remote).await?)? != *original {
            return Err(changed());
        }
        Ok(())
    }
    .await;
    let _ = session.close(handle).await;
    result
}
