//! Desktop-only upload recovery. A record references a private server staging file.
use super::*;
use rustix::fs::{Mode, OFlags, open};
use serde::Serialize;
use std::{fs, io, os::unix::fs::DirBuilderExt, path::Path, time::Instant};
use vibe_ssh_core::UploadPlan;

const MAX_PENDING: usize = 32;

fn io_error(error: io::Error) -> ErrorBody {
    invalid(&format!("上传记录操作失败：{error}"))
}

fn record(home: &Path, id: &str) -> Reply<PathBuf> {
    if !uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        return Err(invalid("上传记录标识无效。"));
    }
    Ok(home.join("uploads").join(format!("{id}.json")))
}

fn load(home: &Path, id: &str) -> Reply<UploadPlan> {
    let file = fs::File::from(
        open(
            record(home, id)?,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| io_error(error.into()))?,
    );
    let meta = file.metadata().map_err(io_error)?;
    if !meta.is_file() || meta.len() > 65_536 {
        return Err(invalid("上传记录已损坏。"));
    }
    serde_json::from_reader(file).map_err(|_| invalid("上传记录已损坏。"))
}

pub(super) fn create(home: &Path, active: &Active<'_>, plan: &UploadPlan) -> Reply<()> {
    let path = record(home, &active.id)?;
    let directory = home.join("uploads");
    let _queue = active.state.0.lock().unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&directory)
        .map_err(io_error)?;
    let used = fs::read_dir(&directory)
        .map_err(io_error)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .take(MAX_PENDING)
        .count();
    if used >= MAX_PENDING {
        return Err(invalid("待续传的上传已满，请继续或移除旧记录。"));
    }
    let data = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
    serde_json::to_writer(data.as_file(), plan).map_err(|_| invalid("保存上传记录失败。"))?;
    data.as_file().sync_all().map_err(io_error)?;
    data.persist_noclobber(path)
        .map_err(|error| io_error(error.error))?;
    Ok(())
}

#[derive(Serialize)]
pub struct PendingUpload {
    id: String,
    host: String,
    name: String,
    total: u64,
    staging: String,
}

#[tauri::command]
pub fn pending_uploads(
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
) -> Reply<Vec<PendingUpload>> {
    let active = state.0.lock().unwrap();
    let entries = match fs::read_dir(kernel.home().join("uploads")) {
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
        if let Ok(plan) = load(kernel.home(), id)
            && let Ok(staging) = plan.staging_path()
        {
            result.push(PendingUpload {
                id: id.into(),
                host: plan.host_id.clone(),
                name: plan.name().into(),
                total: plan.size(),
                staging,
            });
        }
    }
    result.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(result)
}

#[tauri::command]
pub fn discard_upload(kernel: Kernel<'_>, state: State<'_, Transfers>, id: String) -> Reply<()> {
    let active = state.0.lock().unwrap();
    if active.contains_key(&id) {
        return Err(invalid("请先暂停上传。"));
    }
    fs::remove_file(record(kernel.home(), &id)?).map_err(io_error)
}

#[tauri::command]
pub async fn resume_upload<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
    space: String,
    id: String,
) -> Reply<bool> {
    let (active, cancel) = start(&state, id, &space)?;
    let plan = load(kernel.home(), &active.id)?;
    transfer(&app, &kernel, &space, &active, &plan, true, &cancel).await?;
    Ok(true)
}

pub(super) async fn transfer<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kernel: &Service,
    space: &str,
    active: &Active<'_>,
    plan: &UploadPlan,
    resume: bool,
    cancel: &AtomicBool,
) -> Reply<()> {
    let id = &active.id;
    let name = plan.name();
    let total = plan.size();
    let _ = app.emit("file-transfer", json!({"id":id,"name":name,"direction":"upload","status":"running","done":0,"total":total,"resumable":true}));
    let last = Mutex::new(Instant::now());
    let progress = |progress: TransferProgress| {
        let mut last = last.lock().unwrap();
        if last.elapsed().as_millis() >= 100 || progress.done == total {
            update(app, id, name, "upload", progress);
            *last = Instant::now();
        }
    };
    observed_transfer(
        kernel,
        space,
        FileTransferKind::Upload,
        plan.remote.clone(),
        Some(total),
        async {
            kernel
                .resume_upload(space, plan, resume, cancel, progress)
                .await
                .map_err(fail)
        },
    )
    .await?;
    fs::remove_file(record(kernel.home(), id)?)
        .map_err(|error| invalid(&format!("文件已上传，续传记录清理失败：{error}")))
}
