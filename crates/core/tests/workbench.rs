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
    println!("real Linux resources: {resources}");
    service.close(By::Human(&space)).await.unwrap();
}
