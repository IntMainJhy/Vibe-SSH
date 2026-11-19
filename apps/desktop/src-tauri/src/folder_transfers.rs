use super::*;
use rustix::fs::{Mode, OFlags, open};
use serde::Serialize;
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::DirBuilderExt,
    path::Path,
    time::Instant,
};
use vibe_ssh_core::FolderPlan;

const MAX_PENDING: usize = 32;
const MAX_RECORD: u64 = 4 * 1024 * 1024;

fn local_error(error: io::Error) -> ErrorBody {
    invalid(&format!("文件夹续传记录：{error}"))
}

fn record(home: &Path, id: &str) -> Reply<PathBuf> {
    if !uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        return Err(invalid("文件夹续传记录标识无效。"));
    }
    Ok(home.join("folder-transfers").join(format!("{id}.json")))
}

fn load(home: &Path, id: &str) -> Reply<FolderPlan> {
    let fd = open(
        record(home, id)?,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| local_error(err.into()))?;
    let file = fs::File::from(fd);
    let meta = file.metadata().map_err(local_error)?;
    if !meta.is_file() || meta.len() > MAX_RECORD {
        return Err(invalid("文件夹续传记录无效。"));
    }
    serde_json::from_reader(io::BufReader::new(file))
        .map_err(|_| invalid("文件夹续传记录无法读取。"))
}

fn save(home: &Path, active: &Active<'_>, plan: &FolderPlan) -> Reply<()> {
    let path = record(home, &active.id)?;
    let directory = home.join("folder-transfers");
    let _queue = active.state.0.lock().unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&directory)
        .map_err(local_error)?;
    let used = fs::read_dir(&directory)
        .map_err(local_error)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .take(MAX_PENDING)
        .count();
    if used >= MAX_PENDING {
        return Err(invalid("待续传文件夹已达上限，请继续或移除一条记录。"));
    }
    let bytes = serde_json::to_vec(plan).map_err(|_| invalid("保存文件夹计划失败。"))?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(invalid("文件夹续传计划超出大小限制。"));
    }
    let mut data = tempfile::NamedTempFile::new_in(directory).map_err(local_error)?;
    data.write_all(&bytes).map_err(local_error)?;
    data.as_file().sync_all().map_err(local_error)?;
    data.persist_noclobber(path)
        .map_err(|error| local_error(error.error))?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn paused_fixture(
    kernel: &Service,
    state: &Transfers,
    space: &str,
    local: &Path,
    remote: &str,
    upload: bool,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let (active, cancel) = start(state, id.clone(), space).unwrap();
    let plan = kernel
        .plan_folder(space, local, remote, upload, &cancel)
        .await
        .unwrap();
    save(kernel.home(), &active, &plan).unwrap();
    let result = kernel
        .transfer_folder_plan(space, &plan, false, &cancel, |done| {
            if done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .await;
    assert!(result.is_err());
    assert!(cancel.load(Ordering::Relaxed));
    id
}

#[derive(Serialize)]
pub struct PendingFolder {
    id: String,
    name: String,
    host: String,
    direction: &'static str,
    total: u64,
    staging: String,
    folder: bool,
}

fn direction(plan: &FolderPlan) -> &'static str {
    if plan.upload { "upload" } else { "download" }
}

#[tauri::command]
pub async fn pending_folders(
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
) -> Reply<Vec<PendingFolder>> {
    let active: std::collections::BTreeSet<_> = state.0.lock().unwrap().keys().cloned().collect();
    let home = kernel.home().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entries = match fs::read_dir(home.join("folder-transfers")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(local_error(error)),
        };
        let mut pending = Vec::new();
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
                continue;
            };
            if active.contains(id) {
                continue;
            }
            if let Ok(plan) = load(&home, id)
                && let Ok(staging) = plan.staging()
            {
                pending.push(PendingFolder {
                    id: id.into(),
                    name: plan.name().into(),
                    host: plan.host_id.clone(),
                    direction: direction(&plan),
                    total: plan.total(),
                    staging,
                    folder: true,
                });
            }
        }
        pending.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(pending)
    })
    .await
    .map_err(|_| invalid("无法读取待续传文件夹。"))?
}

#[tauri::command]
pub fn discard_folder(kernel: Kernel<'_>, state: State<'_, Transfers>, id: String) -> Reply<()> {
    let active = state.0.lock().unwrap();
    if active.contains_key(&id) {
        return Err(invalid("请先暂停传输。"));
    }
    fs::remove_file(record(kernel.home(), &id)?).map_err(local_error)
}

#[tauri::command]
pub async fn transfer_folder<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
    space: String,
    remote: String,
    id: String,
    upload: bool,
) -> Reply<bool> {
    let (active, cancel) = start(&state, id, &space)?;
    let Some(folder) = rfd::AsyncFileDialog::new()
        .set_title(if upload {
            "选择要上传的文件夹"
        } else {
            "选择下载位置"
        })
        .pick_folder()
        .await
    else {
        return Ok(false);
    };
    let plan = kernel
        .plan_folder(&space, folder.path(), &remote, upload, &cancel)
        .await
        .map_err(fail)?;
    save(kernel.home(), &active, &plan)?;
    transfer(&app, &kernel, &space, &active, &plan, false, &cancel).await?;
    Ok(true)
}

#[tauri::command]
pub async fn resume_folder<R: tauri::Runtime>(
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

async fn transfer<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kernel: &Service,
    space: &str,
    active: &Active<'_>,
    plan: &FolderPlan,
    resume: bool,
    cancel: &AtomicBool,
) -> Reply<()> {
    let id = &active.id;
    let name = plan.name();
    let direction = direction(plan);
    let total = plan.total();
    let _ = app.emit("file-transfer", json!({"id":id,"name":name,"direction":direction,"done":0,"total":total,"resumable":true,"status":"running"}));
    let last = Mutex::new(Instant::now());
    let progress = |done| {
        let mut last = last.lock().unwrap();
        if last.elapsed().as_millis() >= 100 || done == total {
            update(
                app,
                id,
                name,
                direction,
                TransferProgress {
                    done,
                    total: Some(total),
                },
            );
            *last = Instant::now();
        }
    };
    let kind = if plan.upload {
        FileTransferKind::UploadFolder
    } else {
        FileTransferKind::DownloadFolder
    };
    observed_transfer(
        kernel,
        space,
        kind,
        plan.remote.clone(),
        Some(total),
        async {
            kernel
                .transfer_folder_plan(space, plan, resume, cancel, progress)
                .await
                .map_err(fail)
        },
    )
    .await?;
    fs::remove_file(record(kernel.home(), id)?)
        .map_err(|error| invalid(&format!("文件夹已传输完成，续传记录清理失败：{error}")))
}
