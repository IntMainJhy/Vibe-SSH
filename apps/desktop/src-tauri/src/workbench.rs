//! Trusted desktop-only file selection and streaming. These commands are absent from agent IPC.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::Manager;
use vibe_ssh_core::{
    DiffApplyPlan, DiffApplyResult, Directory, TextDocument, TextSaved, TransferProgress,
};
use vibe_ssh_protocol::{FileTransferKind, FileTransferRecord};

#[path = "downloads.rs"]
pub mod downloads;
#[path = "folder_transfers.rs"]
pub mod folders;
#[path = "uploads.rs"]
pub mod uploads;

#[derive(Default)]
pub struct DiffSources(Mutex<BTreeMap<String, DiffSource>>);

struct DiffSource {
    root: PathBuf,
    manifest: vibe_ssh_protocol::Manifest,
    review: Option<(String, DiffApplyPlan)>,
    revision: String,
}

impl DiffSource {
    fn new(root: PathBuf, manifest: vibe_ssh_protocol::Manifest) -> Self {
        Self {
            root,
            manifest,
            review: None,
            revision: String::new(),
        }
    }
}

#[cfg(test)]
impl DiffSources {
    pub fn fixture(&self, id: &str, path: PathBuf, manifest: vibe_ssh_protocol::Manifest) {
        self.0
            .lock()
            .unwrap()
            .insert(id.into(), DiffSource::new(path, manifest));
    }
}

#[tauri::command]
pub async fn choose_diff(
    state: State<'_, DiffSources>,
    excludes: Vec<String>,
) -> Reply<Option<Value>> {
    vibe_ssh_core::diff::rules(&excludes).map_err(fail)?;
    let Some(folder) = rfd::AsyncFileDialog::new()
        .set_title("选择要比较的项目目录")
        .pick_folder()
        .await
    else {
        return Ok(None);
    };
    let path = folder.path().to_path_buf();
    let root = path.clone();
    let manifest =
        tauri::async_runtime::spawn_blocking(move || vibe_ssh_core::diff::scan(&root, &excludes))
            .await
            .map_err(|_| invalid("目录读取任务未完成，请重试。"))?
            .map_err(fail)?;
    let id = uuid::Uuid::new_v4().to_string();
    let result = json!({"id":id,"name":path.display().to_string(),"manifest":manifest});
    let mut sources = state.0.lock().unwrap();
    if sources.len() >= 4 {
        let key = sources.keys().next().unwrap().clone();
        sources.remove(&key);
    }
    sources.insert(id, DiffSource::new(path, manifest));
    Ok(Some(result))
}
#[tauri::command]
pub fn forget_diff(state: State<'_, DiffSources>, id: String) {
    state.0.lock().unwrap().remove(&id);
}
#[tauri::command]
pub async fn rescan_diff(
    state: State<'_, DiffSources>,
    id: String,
    excludes: Vec<String>,
) -> Reply<vibe_ssh_protocol::Manifest> {
    let revision = uuid::Uuid::new_v4().to_string();
    let root = {
        let mut sources = state.0.lock().unwrap();
        let source = sources
            .get_mut(&id)
            .ok_or_else(|| invalid("项目目录已过期，请重新选择。"))?;
        source.review = None;
        source.revision.clone_from(&revision);
        source.root.clone()
    };
    let manifest =
        tauri::async_runtime::spawn_blocking(move || vibe_ssh_core::diff::scan(&root, &excludes))
            .await
            .map_err(|_| invalid("目录读取任务未完成，请重试。"))?
            .map_err(fail)?;
    let mut sources = state.0.lock().unwrap();
    sources
        .get_mut(&id)
        .filter(|source| source.revision == revision)
        .ok_or_else(|| invalid("项目目录已变化，请重新选择。"))?
        .manifest = manifest.clone();
    Ok(manifest)
}
#[tauri::command]
pub async fn compare_directory(
    kernel: Kernel<'_>,
    state: State<'_, DiffSources>,
    id: String,
    selected: Vec<String>,
    space: String,
    remote: String,
) -> Reply<Value> {
    let revision = uuid::Uuid::new_v4().to_string();
    let (root, manifest) = {
        let mut sources = state.0.lock().unwrap();
        let source = sources
            .get_mut(&id)
            .ok_or_else(|| invalid("项目目录已过期，请重新选择。"))?;
        source.review = None;
        source.revision.clone_from(&revision);
        (source.root.clone(), source.manifest.clone())
    };
    let manifest = tauri::async_runtime::spawn_blocking(move || {
        vibe_ssh_core::diff::select(&root, &manifest, &selected)
    })
    .await
    .map_err(|_| invalid("文件校验任务未完成，请重试。"))?
    .map_err(fail)?;
    let (preview, plan) = kernel
        .review_diff(&space, &remote, manifest)
        .await
        .map_err(fail)?;
    let mut response = json!(preview);
    if let Some(plan) = plan {
        let paths: Vec<_> = plan
            .entries()
            .map(|(path, replaces)| json!({"path":path,"replaces":replaces}))
            .collect();
        let mut sources = state.0.lock().unwrap();
        let source = sources
            .get_mut(&id)
            .filter(|source| source.revision == revision)
            .ok_or_else(|| invalid("比较结果已过期，请重新比较。"))?;
        source.review = Some((space, plan));
        response["apply"] = json!({"revision":revision,"paths":paths});
    }
    Ok(response)
}

#[tauri::command]
pub async fn apply_directory(
    kernel: Kernel<'_>,
    state: State<'_, DiffSources>,
    transfers: State<'_, Transfers>,
    id: String,
    revision: String,
    space: String,
) -> Reply<DiffApplyResult> {
    let (_active, cancel) = start(&transfers, uuid::Uuid::new_v4().to_string(), &space)?;
    let (root, plan) = {
        let mut sources = state.0.lock().unwrap();
        let source = sources
            .get_mut(&id)
            .filter(|source| source.revision == revision)
            .filter(|source| {
                source
                    .review
                    .as_ref()
                    .is_some_and(|(target, _)| target == &space)
            })
            .ok_or_else(|| invalid("应用清单已失效，请重新比较。"))?;
        let (_, plan) = source.review.take().expect("review checked");
        (source.root.clone(), plan)
    };
    kernel.apply_diff(plan, root, &cancel).await.map_err(fail)
}

#[derive(Default)]
pub struct Transfers(Mutex<BTreeMap<String, (String, Arc<AtomicBool>)>>);
impl Transfers {
    pub fn active(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}
struct Active<'a> {
    id: String,
    state: &'a Transfers,
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.state.0.lock().unwrap().remove(&self.id);
    }
}
fn start<'a>(
    state: &'a Transfers,
    id: String,
    space: &str,
) -> Reply<(Active<'a>, Arc<AtomicBool>)> {
    let mut entries = state.0.lock().unwrap();
    if entries.contains_key(&id) || id.len() > 80 || id.is_empty() {
        return Err(invalid("invalid transfer id"));
    }
    if entries.values().any(|(target, _)| target == space) {
        return Err(invalid("这个空间已有文件传输正在进行。"));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    entries.insert(id.clone(), (space.into(), cancel.clone()));
    Ok((Active { id, state }, cancel))
}
fn update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
    name: &str,
    direction: &str,
    progress: TransferProgress,
) {
    let _ = app.emit("file-transfer", json!({"id":id,"name":name,"direction":direction,"done":progress.done,"total":progress.total,"status":"running"}));
}

async fn observed_transfer<T>(
    kernel: &Service,
    space: &str,
    kind: FileTransferKind,
    remote: String,
    size: Option<u64>,
    operation: impl Future<Output = Reply<T>>,
) -> Reply<T> {
    let observation = kernel
        .observe_transfer(space, FileTransferRecord { kind, remote, size })
        .await;
    let result = operation.await;
    if let Some(observation) = observation {
        kernel
            .finish_transfer(observation, result.as_ref().err())
            .await;
    }
    result
}

#[tauri::command]
pub async fn files(kernel: Kernel<'_>, space: String, path: String) -> Reply<Directory> {
    kernel.files(&space, &path).await.map_err(fail)
}
#[tauri::command]
pub async fn text_file(kernel: Kernel<'_>, space: String, path: String) -> Reply<TextDocument> {
    kernel.text_file(&space, &path).await.map_err(fail)
}
#[tauri::command]
pub async fn save_text(
    kernel: Kernel<'_>,
    space: String,
    revision: String,
    text: String,
) -> Reply<TextSaved> {
    kernel
        .save_text(By::Human(&space), &revision, &text)
        .await
        .map_err(fail)
}
#[tauri::command]
pub fn forget_text(kernel: Kernel<'_>, space: String, revision: String) -> Reply<()> {
    kernel.forget_text(&space, &revision).map_err(fail)
}

#[tauri::command]
pub async fn resources(
    kernel: Kernel<'_>,
    space: String,
) -> Reply<vibe_ssh_protocol::ResourceSample> {
    kernel.resources(&space, false).await.map_err(fail)
}
#[tauri::command]
pub async fn file_action(
    kernel: Kernel<'_>,
    space: String,
    action: String,
    path: String,
    destination: Option<String>,
) -> Reply<()> {
    kernel
        .file_action(By::Human(&space), &action, &path, destination.as_deref())
        .await
        .map_err(fail)
}
#[tauri::command]
pub fn cancel_transfer(state: State<'_, Transfers>, id: String) {
    if let Some((_, cancel)) = state.0.lock().unwrap().get(&id) {
        cancel.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn upload_files<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kernel: Kernel<'_>,
    state: State<'_, Transfers>,
    space: String,
    directory: String,
    id: String,
) -> Reply<bool> {
    let (active, cancel) = start(&state, id.clone(), &space)?;
    let selected = rfd::AsyncFileDialog::new()
        .set_title("选择要上传的文件")
        .pick_files()
        .await;
    let Some(selected) = selected else {
        return Ok(false);
    };
    if let [file] = selected.as_slice() {
        let plan = kernel
            .plan_upload(&space, &directory, file.path().to_path_buf())
            .await
            .map_err(fail)?;
        uploads::create(kernel.home(), &active, &plan)?;
        uploads::transfer(&app, &kernel, &space, &active, &plan, false, &cancel).await?;
        return Ok(true);
    }
    let mut last_event = std::time::Instant::now();
    for selected in selected {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("传输已取消"));
        }
        let path = selected.path();
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        if !metadata.is_file() {
            return Err(invalid("只上传普通文件，不自动跟随符号链接。"));
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("文件名不是有效的 UTF-8"))?;
        let source = tokio::fs::File::open(path)
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        let remote = format!("{}/{}", directory.trim_end_matches('/'), name);
        update(
            &app,
            &id,
            name,
            "upload",
            TransferProgress {
                done: 0,
                total: Some(metadata.len()),
            },
        );
        // Throttle UI events; file data never crosses Tauri JSON IPC.
        let progress_time = std::sync::Mutex::new(last_event);
        let operation = kernel.upload_file(
            By::Human(&space),
            &remote,
            source,
            metadata.len(),
            &cancel,
            |progress| {
                let mut time = progress_time.lock().unwrap();
                if time.elapsed().as_millis() >= 100 || progress.total == Some(progress.done) {
                    update(&app, &id, name, "upload", progress);
                    *time = std::time::Instant::now();
                }
            },
        );
        observed_transfer(
            &kernel,
            &space,
            FileTransferKind::Upload,
            remote.clone(),
            Some(metadata.len()),
            async { operation.await.map_err(fail) },
        )
        .await?;
        last_event = std::time::Instant::now();
    }
    Ok(true)
}
/// Integration failures are reported without introducing an install button.
pub fn configure<R: tauri::Runtime>(app: &tauri::App<R>, service: &Service) {
    let result = skill::install(
        service.home(),
        &integration_home(),
        std::env::current_exe().ok().as_deref(),
    );
    let report = match result {
        Ok(report) => report,
        Err(error) => json!({"error":error.message}),
    };
    *app.state::<Integration>().0.lock().unwrap() = report;
}
#[derive(Default)]
pub struct Integration(pub Mutex<Value>);
