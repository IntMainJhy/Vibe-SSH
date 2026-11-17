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
async fn sftp_roundtrip_names_no_overwrite_symlinks_and_directory_actions() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .unwrap();
    let space = service
        .open("test", "files", "me", Owner::Human)
        .await
        .unwrap()
        .space;
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
    assert!(service.resources(&space).await.is_err());
    service.close(By::Human(&space)).await.unwrap();
}

#[tokio::test]
async fn transfer_takeover_and_cancellation_never_publish_partial_files_or_restore_old_authority() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    std::fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
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
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::env::var_os("VIBE_SSH_LINUX_VM").is_some());
    let output = std::process::Command::new(root.join("scripts/linux-vm.sh"))
        .arg("hosts")
        .output()
        .unwrap();
    assert!(output.status.success());
    let hosts: Vec<vibe_ssh_core::Host> = serde_json::from_slice(&output.stdout).unwrap();
    let host = hosts.into_iter().find(|h| h.id == "vm").unwrap();
    let home = tempfile::tempdir().unwrap();
    let service = Service::new(home.path().to_owned(), Box::new(|_| None));
    service.save_hosts(std::slice::from_ref(&host)).unwrap();
    let space = service
        .open(&host.id, "monitor", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let resources = service.resources(&space).await.unwrap();
    assert!(
        resources["cpu_percent"]
            .as_f64()
            .is_some_and(|v| (0.0..=100.0).contains(&v))
    );
    assert!(resources["cores"].as_u64().unwrap() > 0);
    assert!(
        resources["memory_total"].as_u64().unwrap()
            > resources["memory_available"].as_u64().unwrap()
    );
    assert!(resources["disk_total"].as_u64().unwrap() > resources["disk_used"].as_u64().unwrap());
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
    println!("real Linux resources: {resources}");
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
