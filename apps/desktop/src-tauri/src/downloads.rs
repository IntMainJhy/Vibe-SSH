//! Desktop-only download recovery. Records never contain credentials or lease tokens.
use super::*;
use rustix::fs::{CWD, Mode, OFlags, RenameFlags, open, renameat_with};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, Metadata, OpenOptions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
    time::Instant,
};
use vibe_ssh_core::DownloadPlan;

const MAX_PENDING: usize = 32;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
struct FileId {
    device: u64,
    inode: u64,
}

impl From<&Metadata> for FileId {
    fn from(meta: &Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
}

#[derive(Deserialize, PartialEq, Eq, Serialize)]
struct DestinationStamp {
    file: FileId,
    size: u64,
    modified: i64,
    nanoseconds: i64,
}

fn destination_stamp(path: &Path) -> io::Result<Option<DestinationStamp>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(DestinationStamp {
            file: (&meta).into(),
            size: meta.len(),
            modified: meta.mtime(),
            nanoseconds: meta.mtime_nsec(),
        })),
        Ok(_) => Err(io::Error::other("保存位置已被其他项目占用。")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Deserialize, Serialize)]
struct Download {
    plan: DownloadPlan,
    destination: PathBuf,
    replace: Option<DestinationStamp>,
    partial: FileId,
}

impl Download {
    fn staging(&self, id: &str) -> PathBuf {
        self.destination
            .with_file_name(format!(".vibe-download-{id}"))
    }

    fn name(&self) -> &str {
        self.plan.remote.rsplit('/').next().unwrap_or_default()
    }

    fn published(&self) -> bool {
        destination_stamp(&self.destination)
            .is_ok_and(|stamp| stamp.is_some_and(|stamp| stamp.file == self.partial))
    }
}

fn check_id(id: &str) -> Reply<()> {
    if uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        return Ok(());
    }
    Err(invalid("下载记录标识无效。"))
}

fn io_error(error: io::Error) -> ErrorBody {
    invalid(&format!("下载文件操作失败：{error}"))
}

fn load(home: &Path, id: &str) -> Reply<Download> {
    check_id(id)?;
    let file = fs::File::from(
        open(
            home.join("downloads").join(format!("{id}.json")),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| io_error(error.into()))?,
    );
    let meta = file.metadata().map_err(io_error)?;
    if !meta.is_file() || meta.len() > 65_536 {
        return Err(invalid("下载记录已损坏。"));
    }
    serde_json::from_reader(file).map_err(|_| invalid("下载记录已损坏。"))
}

fn owned_partial(download: &Download, id: &str) -> io::Result<Option<u64>> {
    match fs::symlink_metadata(download.staging(id)) {
        Ok(meta) if meta.is_file() && FileId::from(&meta) == download.partial => {
            Ok(Some(meta.len()))
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn forget(home: &Path, id: &str, download: &Download) -> Reply<()> {
    if owned_partial(download, id).map_err(io_error)?.is_some() {
        fs::remove_file(download.staging(id)).map_err(io_error)?;
    }
    fs::remove_file(home.join("downloads").join(format!("{id}.json"))).map_err(io_error)?;
    Ok(())
}

fn create(home: &Path, id: &str, path: &Path, plan: DownloadPlan) -> Reply<Download> {
    let directory = home.join("downloads");
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&directory)
        .map_err(io_error)?;
    let name = path.file_name().ok_or_else(|| invalid("保存位置无效。"))?;
    let parent = path.parent().ok_or_else(|| invalid("保存位置无效。"))?;
    let path = parent.canonicalize().map_err(io_error)?.join(name);
    let replace = destination_stamp(&path).map_err(io_error)?;
    let data = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
    let partial = path.with_file_name(format!(".vibe-download-{id}"));
    let staged = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&partial)
        .map_err(io_error)?;
    let persisted = (|| {
        let stamp = staged.metadata().map_err(io_error)?;
        let download = Download {
            plan,
            destination: path,
            replace,
            partial: FileId::from(&stamp),
        };
        serde_json::to_writer(data.as_file(), &download)
            .map_err(|_| invalid("保存下载记录失败。"))?;
        data.as_file().sync_all().map_err(io_error)?;
        data.persist_noclobber(directory.join(format!("{id}.json")))
            .map_err(|error| io_error(error.error))?;
        Ok(download)
    })();
    if persisted.is_err() {
        let _ = fs::remove_file(partial);
    }
    persisted
}

#[derive(Serialize)]
pub struct PendingDownload {
    id: String,
    name: String,
    host: String,
    done: u64,
    total: u64,
}

#[tauri::command]
pub fn pending_downloads(
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
) -> Reply<Vec<PendingDownload>> {
    let active = state.0.lock().unwrap();
    let directory = kernel.home().join("downloads");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    let mut result = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        if active.contains_key(id) {
            continue;
        }
        let Ok(download) = load(kernel.home(), id) else {
            continue;
        };
        if download.published() && forget(kernel.home(), id, &download).is_ok() {
            continue;
        }
        result.push(PendingDownload {
            id: id.into(),
            name: download.name().into(),
            host: download.plan.host_id.clone(),
            done: owned_partial(&download, id).ok().flatten().unwrap_or(0),
            total: download.plan.size,
        });
    }
    result.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(result)
}

#[tauri::command]
pub fn discard_download(kernel: Kernel<'_>, state: State<'_, Transfers>, id: String) -> Reply<()> {
    let active = state.0.lock().unwrap();
    if active.contains_key(&id) {
        return Err(invalid("请先暂停下载。"));
    }
    let download = load(kernel.home(), &id)?;
    forget(kernel.home(), &id, &download)
}

#[tauri::command]
pub async fn download_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
    space: String,
    remote: String,
    id: String,
) -> Reply<bool> {
    check_id(&id)?;
    let (active, cancel) = start(&state, id.clone(), &space)?;
    let plan = kernel.plan_download(&space, &remote).await.map_err(fail)?;
    let Some(selected) = rfd::AsyncFileDialog::new()
        .set_title("保存下载文件")
        .set_file_name(remote.rsplit('/').next().unwrap_or_default())
        .save_file()
        .await
    else {
        return Ok(false);
    };
    let download = {
        let _active = state.0.lock().unwrap();
        let count = fs::read_dir(kernel.home().join("downloads")).map_or(0, |entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .take(MAX_PENDING)
                .count()
        });
        if count >= MAX_PENDING {
            return Err(invalid("待续传的下载已满，请继续或移除旧下载。"));
        }
        create(kernel.home(), &id, selected.path(), plan)?
    };
    transfer(&app, &kernel, &space, &active, &download, &cancel).await?;
    Ok(true)
}

#[tauri::command]
pub async fn resume_download<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
    space: String,
    id: String,
) -> Reply<bool> {
    check_id(&id)?;
    let (active, cancel) = start(&state, id.clone(), &space)?;
    let download = load(kernel.home(), &id)?;
    transfer(&app, &kernel, &space, &active, &download, &cancel).await?;
    Ok(true)
}

async fn transfer<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kernel: &Service,
    space: &str,
    active: &Active<'_>,
    download: &Download,
    cancel: &AtomicBool,
) -> Reply<()> {
    observed_transfer(
        kernel,
        space,
        FileTransferKind::Download,
        download.plan.remote.clone(),
        Some(download.plan.size),
        stream_download(app, kernel, space, active, download, cancel),
    )
    .await
}

async fn stream_download<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kernel: &Service,
    space: &str,
    active: &Active<'_>,
    download: &Download,
    cancel: &AtomicBool,
) -> Reply<()> {
    let id = &active.id;
    if download.published() {
        return forget(kernel.home(), id, download);
    }
    if destination_stamp(&download.destination).map_err(io_error)? != download.replace {
        return Err(invalid("保存位置已变化，请选择新的保存位置重新下载。"));
    }
    let staging = download.staging(id);
    let mut target = tokio::fs::File::from_std(
        open(
            &staging,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| io_error(error.into()))?
        .into(),
    );
    let meta = target.metadata().await.map_err(io_error)?;
    if !meta.is_file() || FileId::from(&meta) != download.partial {
        return Err(invalid("未完成的下载文件已移动或替换，请重新下载。"));
    }
    let time = Mutex::new(Instant::now());
    update(
        app,
        id,
        download.name(),
        "download",
        TransferProgress {
            done: meta.len(),
            total: Some(download.plan.size),
        },
    );
    let result = kernel
        .resume_download(space, &download.plan, &mut target, cancel, |progress| {
            let mut time = time.lock().unwrap();
            if time.elapsed().as_millis() >= 100 || progress.total == Some(progress.done) {
                update(app, id, download.name(), "download", progress);
                *time = Instant::now();
            }
        })
        .await
        .map_err(fail);
    target.sync_all().await.map_err(io_error)?;
    result?;
    let _publishing = active.state.0.lock().unwrap();
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("下载已暂停。"));
    }
    if owned_partial(download, id).map_err(io_error)? != Some(download.plan.size) {
        return Err(invalid("未完成的下载文件已移动或替换，请重新下载。"));
    }
    publish(download, &staging)?;
    let _ = forget(kernel.home(), id, download);
    Ok(())
}

fn publish(download: &Download, staging: &Path) -> Reply<()> {
    if download.replace.is_some() {
        if destination_stamp(&download.destination).map_err(io_error)? != download.replace {
            return Err(invalid("保存位置已变化，请选择新的保存位置重新下载。"));
        }
        fs::rename(staging, &download.destination).map_err(io_error)?;
    } else {
        renameat_with(
            CWD,
            staging,
            CWD,
            &download.destination,
            RenameFlags::NOREPLACE,
        )
        .map_err(|error| io_error(error.into()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn interrupted_download_survives_restart_and_checks_prefix_and_destination() {
        let sshd = crate::sshd::Sshd::start();
        let home = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let destination = local.path().join("中文.bin");
        let remote = sshd.path("payload");
        let content: Vec<_> = (0_u8..251).cycle().take(180_000).collect();
        fs::write(&remote, &content).unwrap();
        let host = sshd.host("box", "id_plain", false);
        let connect = || Service::new(home.path().to_owned(), Box::new(|_| None));
        let service = connect();
        service.save_hosts(&[host]).await.unwrap();
        let first = service
            .open("box", "download", "me", Owner::Human)
            .await
            .unwrap()
            .space;
        let plan = service
            .plan_download(&first, remote.to_str().unwrap())
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let record = create(home.path(), &id, &destination, plan).unwrap();
        let staging = record.staging(&id);
        let open_target = || async {
            tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&staging)
                .await
                .unwrap()
        };
        let cancel = AtomicBool::new(false);
        let mut target = open_target().await;
        assert!(
            service
                .resume_download(&first, &record.plan, &mut target, &cancel, |progress| {
                    if progress.done > 0 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                })
                .await
                .is_err()
        );
        target.flush().await.unwrap();
        let partial = fs::read(&staging).unwrap();
        assert!(!partial.is_empty() && partial.len() < content.len());
        assert_eq!(partial, content[..partial.len()]);
        assert!(!destination.exists());
        service.close(By::Human(&first)).await.unwrap();
        drop((target, record, service));

        let restored = load(home.path(), &id).unwrap();
        let service = connect();
        let second = service
            .open("box", "recover", "me", Owner::Human)
            .await
            .unwrap()
            .space;
        let remote_content = fs::File::options()
            .read(true)
            .write(true)
            .open(&remote)
            .unwrap();
        let mtime = remote_content.metadata().unwrap().modified().unwrap();
        let mut changed = content.clone();
        changed[0] ^= 1;
        fs::write(&remote, &changed).unwrap();
        remote_content.set_modified(mtime).unwrap();
        cancel.store(false, Ordering::Relaxed);
        let mut target = open_target().await;
        let mismatch = service
            .resume_download(&second, &restored.plan, &mut target, &cancel, |_| {})
            .await
            .unwrap_err();
        assert!(mismatch.message.contains("内容校验失败"));
        assert_eq!(fs::read(&staging).unwrap(), partial);
        fs::write(&remote, &content).unwrap();
        remote_content.set_modified(mtime).unwrap();
        service
            .resume_download(&second, &restored.plan, &mut target, &cancel, |_| {})
            .await
            .unwrap();
        assert_eq!(fs::read(&staging).unwrap(), content);
        fs::write(&destination, b"independent file").unwrap();
        assert!(publish(&restored, &staging).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"independent file");
        fs::remove_file(&destination).unwrap();
        publish(&restored, &staging).unwrap();
        assert!(restored.published());
        forget(home.path(), &id, &restored).unwrap();
        assert!(!staging.exists());
        assert!(load(home.path(), &id).is_err());
        assert_eq!(fs::read(&destination).unwrap(), content);
    }
}
