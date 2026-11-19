#[path = "../../cli/tests/support/sshd.rs"]
mod sshd;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::AsyncWriteExt;
use vibe_ssh_core::{By, ErrorCode, Service};
use vibe_ssh_protocol::Owner;

#[tokio::test]
async fn single_file_upload_resumes_only_after_confirming_host_source_prefix_and_destination() {
    use std::fs;
    use vibe_ssh_core::UploadPlan;
    use vibe_ssh_protocol::MAX_INPUT;

    let sshd = sshd::Sshd::start();
    let home = tempfile::tempdir().unwrap();
    let create_service = || Service::new(home.path().to_path_buf(), Box::new(|_| None));
    let service = create_service();
    service
        .save_hosts(&[
            sshd.host("test", "id_plain", true),
            sshd.host("other", "id_plain", false),
        ])
        .await
        .unwrap();
    let space = service
        .open("test", "upload", "fixture", Owner::Human)
        .await
        .unwrap()
        .space;
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("续传 中文.bin");
    let bytes: Vec<_> = (0_u64..150_000)
        .map(|i| (i.wrapping_mul(33) ^ (i / 99)) as u8)
        .collect();
    fs::write(&source, &bytes).unwrap();
    let remote = sshd.path("upload-destination");
    fs::create_dir(&remote).unwrap();
    let remote = remote.canonicalize().unwrap();
    let plan = service
        .plan_upload(&space, remote.to_str().unwrap(), source.clone())
        .await
        .unwrap();
    let staged = std::path::PathBuf::from(plan.staging_path().unwrap());
    let target = remote.join(source.file_name().unwrap());
    let cancel = AtomicBool::new(false);
    assert!(
        service
            .resume_upload(&space, &plan, false, &cancel, |progress| {
                if progress.done > 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
            })
            .await
            .is_err()
    );
    assert_eq!(fs::read(&staged).unwrap(), bytes[..MAX_INPUT]);
    assert!(!target.exists());
    let plan: UploadPlan = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    service.close(By::Human(&space)).await.unwrap();
    drop(service);
    let service = create_service();
    let cancel = AtomicBool::new(false);
    let wrong = service
        .open("other", "wrong", "fixture", Owner::Human)
        .await
        .unwrap()
        .space;
    assert!(
        service
            .resume_upload(&wrong, &plan, true, &cancel, |_| {})
            .await
            .is_err()
    );
    let space = service
        .open("test", "upload", "fixture", Owner::Agent)
        .await
        .unwrap()
        .space;
    assert_eq!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .unwrap_err()
            .code,
        ErrorCode::ControlRevoked
    );
    service.takeover(&space).await.unwrap();

    fs::write(&target, "unrelated").unwrap();
    assert!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"unrelated");
    fs::remove_file(&target).unwrap();

    fs::write(&staged, vec![0xff; MAX_INPUT]).unwrap();
    assert_eq!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    assert_eq!(fs::read(&staged).unwrap(), vec![0xff; MAX_INPUT]);
    fs::remove_file(&staged).unwrap();
    std::os::unix::fs::symlink(&source, &staged).unwrap();
    assert!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    fs::remove_file(&staged).unwrap();
    assert!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .is_err()
    );
    assert!(!target.exists());
    fs::write(&staged, &bytes[..MAX_INPUT]).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    let first = std::sync::atomic::AtomicU64::new(u64::MAX);
    use vibe_ssh_protocol::{FileTransferKind, FileTransferRecord, JobStatus};
    let transfer = FileTransferRecord {
        kind: FileTransferKind::Upload,
        remote: plan.remote.clone(),
        size: Some(plan.size()),
    };
    let observation = service
        .observe_transfer(&space, transfer.clone())
        .await
        .unwrap();
    service
        .resume_upload(&space, &plan, true, &cancel, |p| {
            let _ = first.compare_exchange(u64::MAX, p.done, Ordering::Relaxed, Ordering::Relaxed);
        })
        .await
        .unwrap();
    service.finish_transfer(observation, None).await;
    assert_eq!(first.load(Ordering::Relaxed), MAX_INPUT as u64);
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(!staged.exists());
    assert!(
        service
            .resume_upload(&space, &plan, true, &cancel, |_| {})
            .await
            .is_err()
    );

    fs::remove_file(&target).unwrap();
    let plan = service
        .plan_upload(&space, remote.to_str().unwrap(), source.clone())
        .await
        .unwrap();
    fs::write(&source, vec![0xff; bytes.len()]).unwrap();
    assert_eq!(
        service
            .resume_upload(&space, &plan, false, &cancel, |_| {})
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    assert!(!target.exists());
    let interrupted = service.observe_transfer(&space, transfer).await.unwrap();
    let history = service.history_list(0, 20, false).await.unwrap();
    let succeeded = history
        .records
        .iter()
        .find(|record| record.status == JobStatus::Succeeded)
        .unwrap();
    assert_eq!(succeeded.origin, Owner::Human);
    assert_eq!(succeeded.transfer.as_ref().unwrap().remote, plan.remote);
    assert!(
        service
            .history_list(0, 20, true)
            .await
            .unwrap()
            .records
            .is_empty()
    );
    let running = history
        .records
        .iter()
        .find(|record| record.status == JobStatus::Running)
        .unwrap()
        .record
        .clone();
    drop(interrupted);
    drop(service);
    let history = create_service()
        .history_get(&running, 8192, false)
        .await
        .unwrap();
    assert_eq!(history.record.status, JobStatus::Unknown);
    assert_eq!(history.record.reason.as_deref(), Some("client_restarted"));
    assert!(history.record.transfer.is_some());
}

#[tokio::test]
async fn folders_round_trip_and_never_replace_existing_or_publish_a_symlink_tree() {
    let sshd = sshd::Sshd::start();
    let home = tempfile::tempdir().unwrap();
    let service = Service::new(home.path().to_owned(), Box::new(|_| None));
    service
        .save_hosts(&[sshd.host("test", "id_plain", false)])
        .await
        .unwrap();
    let space = service
        .open("test", "folders", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("项目 '文件'");
    let destination = sshd.path("folder-destination");
    let downloaded = scratch.path().join("downloaded");
    std::fs::create_dir_all(source.join("空目录")).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::create_dir(&downloaded).unwrap();
    std::fs::write(source.join("测试.txt"), b"round-trip\0bytes").unwrap();
    let cancel = AtomicBool::new(false);
    let parent = destination.to_str().unwrap();
    service
        .upload_folder(&space, &source, parent, &cancel, |_| {})
        .await
        .unwrap();
    let remote = destination.join(source.file_name().unwrap());
    service
        .download_folder(
            &space,
            remote.to_str().unwrap(),
            &downloaded,
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
    let copy = downloaded.join(source.file_name().unwrap());
    assert!(copy.join("空目录").is_dir());
    assert_eq!(
        std::fs::read(copy.join("测试.txt")).unwrap(),
        b"round-trip\0bytes"
    );
    assert!(
        service
            .upload_folder(&space, &source, parent, &cancel, |_| {})
            .await
            .is_err()
    );
    assert!(
        service
            .download_folder(
                &space,
                remote.to_str().unwrap(),
                &downloaded,
                &cancel,
                |_| {}
            )
            .await
            .is_err()
    );
    let links = scratch.path().join("links");
    std::fs::create_dir(&links).unwrap();
    std::os::unix::fs::symlink(source.join("测试.txt"), links.join("link")).unwrap();
    assert!(
        service
            .upload_folder(&space, &links, parent, &cancel, |_| {})
            .await
            .is_err()
    );
    assert!(!destination.join("links").exists());
    let linked_folder = destination.join("directory-link");
    std::os::unix::fs::symlink(&remote, &linked_folder).unwrap();
    let linked_path = format!("{}/", linked_folder.display());
    assert!(
        service
            .download_folder(&space, &linked_path, &downloaded, &cancel, |_| {})
            .await
            .is_err()
    );
    let cancelled = scratch.path().join("cancelled");
    std::fs::create_dir(&cancelled).unwrap();
    std::fs::write(cancelled.join("payload"), b"not published").unwrap();
    assert!(
        service
            .upload_folder(&space, &cancelled, parent, &cancel, |_| cancel
                .store(true, Ordering::Relaxed))
            .await
            .is_err()
    );
    assert!(!destination.join("cancelled").exists());
}

#[tokio::test]
async fn folder_upload_and_download_resume_verified_bytes_after_restart() {
    use std::{fs, path::PathBuf};
    use vibe_ssh_core::FolderPlan;
    use vibe_ssh_protocol::MAX_INPUT;

    let sshd = sshd::Sshd::start();
    let home = tempfile::tempdir().unwrap();
    let service = || Service::new(home.path().to_owned(), Box::new(|_| None));
    async fn connect(service: &Service) -> String {
        service
            .open("files", "folder resume", "fixture", Owner::Human)
            .await
            .unwrap()
            .space
    }
    let initial = service();
    initial
        .save_hosts(&[sshd.host("files", "id_plain", true)])
        .await
        .unwrap();
    let id = connect(&initial).await;
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("项目 续传");
    let download = scratch.path().join("下载位置");
    let remote_parent = sshd.path("folder-resume");
    fs::create_dir_all(source.join("空目录")).unwrap();
    fs::create_dir(&download).unwrap();
    fs::create_dir(&remote_parent).unwrap();
    let name = "数据.bin";
    let content: Vec<_> = (0..170_000_u64).map(|n| (n ^ (n / 71)) as u8).collect();
    fs::write(source.join(name), &content).unwrap();
    let pause = AtomicBool::new(false);
    let upload = initial
        .plan_folder(&id, &source, remote_parent.to_str().unwrap(), true, &pause)
        .await
        .unwrap();
    let remote = PathBuf::from(&upload.remote);
    let part = PathBuf::from(upload.staging().unwrap());
    let cancel_after_bytes = |done| {
        if done > 0 {
            pause.store(true, Ordering::Relaxed)
        }
    };
    assert!(
        initial
            .transfer_folder_plan(&id, &upload, false, &pause, cancel_after_bytes)
            .await
            .is_err()
    );
    assert!(!remote.exists());
    assert_eq!(fs::read(part.join(name)).unwrap(), content[..MAX_INPUT]);
    initial.close(By::Human(&id)).await.unwrap();
    drop(initial);

    let persisted: FolderPlan =
        serde_json::from_value(serde_json::to_value(&upload).unwrap()).unwrap();
    let next = service();
    let id = connect(&next).await;
    pause.store(false, Ordering::Relaxed);
    fs::create_dir(&remote).unwrap();
    assert!(
        next.transfer_folder_plan(&id, &persisted, true, &pause, |_| {})
            .await
            .is_err()
    );
    fs::remove_dir(&remote).unwrap();
    fs::write(part.join(name), vec![0xff; MAX_INPUT]).unwrap();
    assert_eq!(
        next.transfer_folder_plan(&id, &persisted, true, &pause, |_| {})
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    fs::write(part.join(name), &content[..MAX_INPUT]).unwrap();
    let first = std::sync::atomic::AtomicU64::new(u64::MAX);
    let remember = |done| {
        let _ = first.compare_exchange(u64::MAX, done, Ordering::Relaxed, Ordering::Relaxed);
    };
    next.transfer_folder_plan(&id, &persisted, true, &pause, remember)
        .await
        .unwrap();
    assert_eq!(first.load(Ordering::Relaxed), MAX_INPUT as u64);
    assert_eq!(fs::read(remote.join(name)).unwrap(), content);
    assert!(!part.exists());

    let folder = next
        .plan_folder(&id, &download, remote.to_str().unwrap(), false, &pause)
        .await
        .unwrap();
    let local = download.join(source.file_name().unwrap());
    let part = PathBuf::from(folder.staging().unwrap());
    assert!(
        next.transfer_folder_plan(&id, &folder, false, &pause, cancel_after_bytes)
            .await
            .is_err()
    );
    assert!(!local.exists());
    assert_eq!(fs::read(part.join(name)).unwrap(), content[..MAX_INPUT]);
    next.close(By::Human(&id)).await.unwrap();
    drop(next);

    let folder: FolderPlan =
        serde_json::from_value(serde_json::to_value(&folder).unwrap()).unwrap();
    let recovered = service();
    let id = connect(&recovered).await;
    pause.store(false, Ordering::Relaxed);
    let unexpected = part.join("意外链接");
    std::os::unix::fs::symlink(&source, &unexpected).unwrap();
    assert!(
        recovered
            .transfer_folder_plan(&id, &folder, true, &pause, |_| {})
            .await
            .is_err()
    );
    fs::remove_file(unexpected).unwrap();
    first.store(u64::MAX, Ordering::Relaxed);
    recovered
        .transfer_folder_plan(&id, &folder, true, &pause, remember)
        .await
        .unwrap();
    assert_eq!(first.load(Ordering::Relaxed), MAX_INPUT as u64);
    assert_eq!(fs::read(local.join(name)).unwrap(), content);
    assert!(local.join("空目录").is_dir());
    assert!(!part.exists());
}

#[tokio::test]
async fn sftp_roundtrip_names_no_overwrite_symlinks_and_directory_actions() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .await
        .unwrap();
    let space = service
        .open("test", "files", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let filesystem = service.files(&space, "/").await.unwrap();
    assert_eq!(filesystem.path, "/");
    let tmp = filesystem
        .entries
        .iter()
        .find(|e| e.path == "/tmp")
        .unwrap();
    let temporary = service.files(&space, &tmp.path).await.unwrap();
    assert_eq!(
        temporary.path,
        std::fs::canonicalize("/tmp").unwrap().to_str().unwrap()
    );
    let root = sshd.path("远端 空格 '$`;");
    std::fs::create_dir(&root).unwrap();
    let root = root.to_str().unwrap();
    let cancel = AtomicBool::new(false);
    let content = "中文🚀\0".repeat(250_000).into_bytes();
    let remote = format!("{root}/data '文件.bin");
    service
        .upload_file(
            By::Human(&space),
            &remote,
            content.as_slice(),
            content.len() as u64,
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&remote).unwrap(), content);
    let mut copy = Vec::new();
    service
        .download_file(&space, &remote, &mut copy, &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(copy, content);
    service
        .file_action(By::Human(&space), "chmod", &remote, Some("640"))
        .await
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&remote).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        service
            .file_action(By::Human(&space), "chmod", &remote, Some("888"))
            .await
            .is_err()
    );
    let denied = service
        .upload_file(
            By::Human(&space),
            &remote,
            &b"replace"[..],
            7,
            &cancel,
            |_| {},
        )
        .await;
    assert!(denied.is_err());
    assert_eq!(
        std::fs::read(&remote).unwrap(),
        content,
        "existing file is never replaced"
    );
    let symlink = format!("{root}/link");
    std::os::unix::fs::symlink(&remote, &symlink).unwrap();
    assert!(
        service
            .file_action(By::Human(&space), "chmod", &symlink, Some("600"))
            .await
            .is_err()
    );
    assert_eq!(
        service
            .download_file(&space, &symlink, &mut Vec::new(), &cancel, |_| {})
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let list = service.files(&space, root).await.unwrap();
    assert!(
        list.entries
            .iter()
            .any(|file| file.name == "data '文件.bin" && file.kind == "file")
    );
    assert!(list.entries.iter().any(|file| file.kind == "symlink"));
    let folder = format!("{root}/新建");
    service
        .file_action(By::Human(&space), "mkdir", &folder, None)
        .await
        .unwrap();
    let moved = format!("{root}/moved");
    service
        .file_action(By::Human(&space), "rename", &folder, Some(&moved))
        .await
        .unwrap();
    service
        .file_action(By::Human(&space), "delete", &moved, None)
        .await
        .unwrap();
    assert!(!std::path::Path::new(&moved).exists());
    assert!(
        service
            .file_action(By::Human(&space), "delete", root, None)
            .await
            .is_err(),
        "no recursive delete"
    );
    let empty = format!("{root}/empty");
    service
        .upload_file(By::Human(&space), &empty, &b""[..], 0, &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::metadata(empty).unwrap().len(), 0);
    let large = format!("{root}/large");
    std::fs::create_dir(&large).unwrap();
    for index in 0..5005 {
        std::fs::write(format!("{large}/{index:05}"), "").unwrap();
    }
    let bounded = service.files(&space, &large).await.unwrap();
    assert!(!bounded.entries.is_empty() && bounded.entries.len() <= 5000);
    assert!(serde_json::to_vec(&bounded).unwrap().len() < 1_048_576);
    assert!(bounded.truncated);
    assert!(
        service
            .files(&space, &format!("{root}/missing"))
            .await
            .is_err()
    );
    // An unsupported OS must return a visible error, never fabricated zero usage.
    assert!(service.resources(&space, false).await.is_err());
    service.close(By::Human(&space)).await.unwrap();
}

#[tokio::test]
async fn transfer_takeover_and_cancellation_never_publish_partial_files_or_restore_old_authority() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .await
        .unwrap();
    let view = service
        .open("test", "upload", "test", Owner::Agent)
        .await
        .unwrap();
    let lease = view.control.lease.unwrap();
    let remote = sshd.path("incomplete").to_str().unwrap().to_owned();
    let (mut producer, source) = tokio::io::duplex(32_768);
    let delivered = Arc::new(tokio::sync::Notify::new());
    let upload = {
        let (service, lease, delivered, remote) = (
            service.clone(),
            lease.clone(),
            delivered.clone(),
            remote.clone(),
        );
        tokio::spawn(async move {
            service
                .upload_file(
                    By::Lease(&lease),
                    &remote,
                    source,
                    65_536,
                    &AtomicBool::new(false),
                    |_| delivered.notify_one(),
                )
                .await
        })
    };
    producer.write_all(&vec![b'a'; 32_768]).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), delivered.notified())
        .await
        .unwrap();
    service.takeover(&view.space).await.unwrap();
    producer.write_all(&vec![b'b'; 32_768]).await.unwrap();
    drop(producer);
    assert_eq!(
        upload.await.unwrap().unwrap_err().code,
        ErrorCode::ControlRevoked
    );
    assert!(!std::path::Path::new(&remote).exists());
    service.release(&view.space).await.unwrap();
    assert_eq!(
        service
            .upload_file(
                By::Lease(&lease),
                &remote,
                &b"x"[..],
                1,
                &AtomicBool::new(false),
                |_| {}
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::StaleLease
    );
    service.takeover(&view.space).await.unwrap();
    let cancelled = AtomicBool::new(false);
    let data = vec![b'x'; 100_000];
    let result = service
        .upload_file(
            By::Human(&view.space),
            &remote,
            data.as_slice(),
            data.len() as u64,
            &cancelled,
            |_| cancelled.store(true, Ordering::Relaxed),
        )
        .await;
    assert!(result.is_err());
    assert!(!std::path::Path::new(&remote).exists());
    service.close(By::Human(&view.space)).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the disposable Debian VM; VIBE_SSH_LINUX_VM=target/vm"]
async fn linux_resources_and_files_use_real_remote_data() {
    let vm = std::path::PathBuf::from(
        std::env::var_os("VIBE_SSH_LINUX_VM").expect("set VIBE_SSH_LINUX_VM to target/vm"),
    );
    let root = vm
        .parent()
        .and_then(std::path::Path::parent)
        .expect("target/vm");
    let output = std::process::Command::new(root.join("scripts/linux-vm.sh"))
        .arg("hosts")
        .output()
        .unwrap();
    assert!(output.status.success());
    let hosts: Vec<vibe_ssh_core::Host> = serde_json::from_slice(&output.stdout).unwrap();
    let host = hosts.into_iter().find(|h| h.id == "vm").unwrap();
    let home = tempfile::tempdir().unwrap();
    let service = Service::new(home.path().to_owned(), Box::new(|_| None));
    service
        .save_hosts(std::slice::from_ref(&host))
        .await
        .unwrap();
    let space = service
        .open(&host.id, "monitor", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let resources = service.resources(&space, false).await.unwrap();
    assert!(
        resources
            .cpu_percent
            .is_some_and(|v| (0.0..=100.0).contains(&v))
    );
    assert!(resources.cores > 0);
    assert!(resources.memory_total > resources.memory_available);
    assert!(resources.disk_total > resources.disk_used);
    let files = service.files(&space, "/proc").await.unwrap();
    assert!(files.entries.iter().any(|e| e.name == "stat"));
    let folder = format!("/tmp/vibe-editor-{}", uuid::Uuid::new_v4().simple());
    service
        .file_action(By::Human(&space), "mkdir", &folder, None)
        .await
        .unwrap();
    let case = {
        let (service, space, folder) = (service.clone(), space.clone(), folder.clone());
        tokio::spawn(async move {
            let remote = format!("{folder}/配置.txt");
            let initial = "\u{feff}service=old\r\n";
            service
                .upload_file(
                    By::Human(&space),
                    &remote,
                    initial.as_bytes(),
                    initial.len() as u64,
                    &AtomicBool::new(false),
                    |_| {},
                )
                .await
                .unwrap();
            let doc = service.text_file(&space, &remote).await.unwrap();
            assert!(doc.save_supported);
            let changed = "\u{feff}service=部署完成🚀\r\n";
            let saved = service
                .save_text(By::Human(&space), &doc.revision, changed)
                .await
                .unwrap();
            let mut data = Vec::new();
            service
                .download_file(&space, &remote, &mut data, &AtomicBool::new(false), |_| {})
                .await
                .unwrap();
            assert_eq!(data, changed.as_bytes());
            let project = tempfile::tempdir().unwrap();
            std::fs::write(project.path().join("配置.txt"), changed).unwrap();
            let manifest = vibe_ssh_core::diff::scan(project.path(), &[]).unwrap();
            let selected =
                vibe_ssh_core::diff::select(project.path(), &manifest, &["配置.txt".into()])
                    .unwrap();
            let preview = service
                .diff(&space, &folder, selected, false)
                .await
                .unwrap();
            assert_eq!(
                preview
                    .entries
                    .iter()
                    .find(|e| e.path == "配置.txt")
                    .unwrap()
                    .status,
                "same"
            );
            let mut backup = Vec::new();
            service
                .download_file(
                    &space,
                    saved.backup.as_ref().unwrap(),
                    &mut backup,
                    &AtomicBool::new(false),
                    |_| {},
                )
                .await
                .unwrap();
            assert_eq!(backup, initial.as_bytes());
            assert!(
                service
                    .save_text(By::Human(&space), &doc.revision, "old")
                    .await
                    .is_err()
            );
            service
                .forget_text(&space, &saved.document.revision)
                .unwrap();
        })
    }
    .await;
    // Cleanup also runs if the test task panicked, and only touches its own UUID directory.
    if let Ok(list) = service.files(&space, &folder).await {
        for entry in list.entries {
            service
                .file_action(By::Human(&space), "delete", &entry.path, None)
                .await
                .unwrap();
        }
        service
            .file_action(By::Human(&space), "delete", &folder, None)
            .await
            .unwrap();
    }
    case.unwrap();
    println!("real Linux resources: {resources:?}");
    service.close(By::Human(&space)).await.unwrap();
}

#[tokio::test]
async fn editor_versions_preserve_original_on_conflicts_and_save_with_backup() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .await
        .unwrap();
    let space = service
        .open("test", "editor", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let path = sshd.path("配置 '文件.txt");
    std::fs::write(&path, "one").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let baseline = std::fs::metadata(&path).unwrap();
    let remote = path.to_str().unwrap();
    let first = service.text_file(&space, remote).await.unwrap();
    assert!(first.save_supported);
    assert_eq!(first.text, "one");
    assert_eq!(service.open_documents(), 1);
    // Same bytes and second-resolution metadata are not a sufficient version check.
    std::fs::write(&path, "two").unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(baseline.modified().unwrap()))
        .unwrap();
    assert_eq!(
        service
            .save_text(By::Human(&space), &first.revision, "new")
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    let fresh = service.text_file(&space, remote).await.unwrap();
    let other = service.text_file(&space, remote).await.unwrap();
    let saved = service
        .save_text(By::Human(&space), &fresh.revision, "部署完成🚀\r\n")
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "部署完成🚀\r\n");
    assert_eq!(
        std::fs::read_to_string(saved.backup.as_ref().unwrap()).unwrap(),
        "two"
    );
    let metadata = std::fs::metadata(&path).unwrap();
    assert_eq!(metadata.mode() & 0o7777, baseline.mode() & 0o7777);
    assert_eq!(
        (metadata.uid(), metadata.gid()),
        (baseline.uid(), baseline.gid())
    );
    assert!(
        service
            .save_text(By::Human(&space), &fresh.revision, "replay")
            .await
            .is_err()
    );
    assert_eq!(
        service
            .save_text(By::Human(&space), &other.revision, "stale")
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    let same = service
        .save_text(
            By::Human(&space),
            &saved.document.revision,
            &saved.document.text,
        )
        .await
        .unwrap();
    assert!(
        same.backup.is_none(),
        "unchanged save performs no remote write"
    );
    service
        .forget_text(&space, &same.document.revision)
        .unwrap();
    // Type and permission changes also invalidate a version.
    let permissions = service.text_file(&space, remote).await.unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        service
            .save_text(By::Human(&space), &permissions.revision, "overwrite")
            .await
            .unwrap_err()
            .code,
        ErrorCode::FileChanged
    );
    let removed = service.text_file(&space, remote).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        service
            .save_text(By::Human(&space), &removed.revision, "recreate")
            .await
            .is_err()
    );
    assert!(!path.exists());
    let target = sshd.path("other");
    std::fs::write(&target, "other").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(service.text_file(&space, remote).await.is_err());
    assert!(
        service
            .save_text(By::Human(&space), &removed.revision, "follow")
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "other");
    service.close(By::Human(&space)).await.unwrap();
    assert_eq!(service.open_documents(), 0);
}

#[tokio::test]
async fn editor_rejects_binary_large_expired_and_control_periods() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .await
        .unwrap();
    let agent = service
        .open("test", "editor", "test", Owner::Agent)
        .await
        .unwrap();
    let lease = agent.control.lease.unwrap();
    let space = &agent.space;
    let path = sshd.path("bounded.txt");
    let remote = path.to_str().unwrap();
    for data in [vec![0, 1, 2], vec![255], vec![b'a'; 1_048_577]] {
        std::fs::write(&path, data).unwrap();
        assert!(service.text_file(space, remote).await.is_err());
    }
    std::fs::write(&path, "original").unwrap();
    service.takeover(space).await.unwrap();
    let draft = service.text_file(space, remote).await.unwrap();
    assert_eq!(
        service
            .save_text(By::Lease(&lease), &draft.revision, "agent stale")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ControlRevoked
    );
    service.release(space).await.unwrap();
    assert_eq!(
        service
            .save_text(By::Human(space), &draft.revision, "user stale")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ControlRevoked
    );
    service.takeover(space).await.unwrap();
    assert_eq!(
        service
            .save_text(By::Human(space), &draft.revision, "old period")
            .await
            .unwrap_err()
            .code,
        ErrorCode::StaleLease
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    let oldest = service.text_file(space, remote).await.unwrap();
    for _ in 0..8 {
        service.text_file(space, remote).await.unwrap();
    }
    assert_eq!(service.open_documents(), 8);
    assert_eq!(
        service
            .save_text(By::Human(space), &oldest.revision, "evicted")
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let current = service.text_file(space, remote).await.unwrap();
    assert!(
        service
            .save_text(By::Human(space), &current.revision, &"x".repeat(1_048_577))
            .await
            .is_err()
    );
    let other = service
        .open("test", "other", "me", Owner::Human)
        .await
        .unwrap();
    assert!(
        service
            .save_text(By::Human(&other.space), &current.revision, "wrong space")
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    service.close(By::Human(space)).await.unwrap();
    service.close(By::Human(&other.space)).await.unwrap();
}
