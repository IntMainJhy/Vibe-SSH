#[path = "../../cli/tests/support/sshd.rs"]
mod sshd;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::AtomicBool,
};
use vibe_ssh_core::{Service, diff};
use vibe_ssh_protocol::Owner;

#[tokio::test]
async fn preview_reads_only_selected_regular_files_and_keeps_personal_spaces_private() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("test", "id_plain", true)])
        .await
        .unwrap();
    let agent = service
        .open("test", "diff", "test", Owner::Agent)
        .await
        .unwrap()
        .space;
    let personal = service
        .open("test", "mine", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let local = sshd.path("本地 '$");
    let remote = sshd.path("远端 '`;");
    fs::create_dir(&local).unwrap();
    fs::create_dir(&remote).unwrap();
    for root in [&local, &remote] {
        fs::write(root.join("相同.txt"), "中文相同\n").unwrap();
        fs::write(
            root.join("变化.txt"),
            if *root == local { "before" } else { "after!" },
        )
        .unwrap();
        fs::write(root.join("未选择.txt"), "do not read me").unwrap();
        fs::write(root.join(".env"), [0xff, 0xfe, 0]).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/隐藏"), "not enumerated").unwrap();
        symlink(root.join("相同.txt"), root.join("link")).unwrap();
    }
    fs::write(local.join("local-only"), "local").unwrap();
    fs::write(remote.join("remote-only"), "remote").unwrap();
    fs::write(local.join("conflict"), "file").unwrap();
    fs::create_dir(remote.join("conflict")).unwrap();
    let metadata = diff::scan(&local, &[]).unwrap();
    // Metadata may be inspected, but ignored and unselected content must never be opened.
    for root in [&local, &remote] {
        fs::set_permissions(root.join("未选择.txt"), fs::Permissions::from_mode(0o0)).unwrap();
        fs::set_permissions(root.join(".env"), fs::Permissions::from_mode(0o0)).unwrap();
    }
    assert!(!metadata.entries.iter().any(|e| e.path.contains("隐藏")));
    assert!(metadata.entries.iter().all(|e| e.digest.is_none()));
    assert!(diff::select(&local, &metadata, &[".env".into()]).is_err());
    assert!(diff::select(&local, &metadata, &["link".into()]).is_err());
    assert!(diff::select(&local, &metadata, &["../outside".into()]).is_err());
    let selected =
        diff::select(&local, &metadata, &["相同.txt".into(), "变化.txt".into()]).unwrap();
    let preview = service
        .diff(&agent, remote.to_str().unwrap(), selected.clone(), true)
        .await
        .unwrap();
    let status = |path: &str| {
        preview
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap()
            .status
            .as_str()
    };
    for (path, expected) in [
        ("相同.txt", "same"),
        ("变化.txt", "changed"),
        ("未选择.txt", "unverified"),
        (".env", "excluded"),
        ("link", "unsupported"),
        ("local-only", "local_only"),
        ("remote-only", "remote_only"),
        ("conflict", "type_conflict"),
    ] {
        assert_eq!(status(path), expected);
    }
    assert!(!preview.truncated);
    assert!(
        service
            .diff(&personal, remote.to_str().unwrap(), selected.clone(), true)
            .await
            .is_err()
    );
    let mut truncated = selected.clone();
    truncated.truncated = true;
    let result = service
        .diff(&agent, remote.to_str().unwrap(), truncated, true)
        .await
        .unwrap();
    assert!(
        !result
            .entries
            .iter()
            .any(|e| ["local_only", "remote_only"].contains(&e.status.as_str())),
        "partial listing cannot prove absence"
    );
    let mut forged = selected;
    forged.entries[0].path = "/etc/passwd".into();
    assert!(
        service
            .diff(&agent, remote.to_str().unwrap(), forged, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn apply_review_checks_both_sides_backs_up_and_requires_current_human_control() {
    let sshd = sshd::Sshd::start();
    let service = Service::new(sshd.path("service"), Box::new(|_| None));
    fs::create_dir(service.home()).unwrap();
    service
        .save_hosts(&[sshd.host("review", "id_plain", true)])
        .await
        .unwrap();
    let space = service
        .open("review", "review", "fixture", Owner::Agent)
        .await
        .unwrap()
        .space;
    service.takeover(&space).await.unwrap();
    let project = sshd.path("发布项目");
    let remote = sshd.path("服务器目录");
    fs::create_dir_all(project.join("assets/子目录")).unwrap();
    fs::create_dir(&remote).unwrap();
    fs::write(project.join("assets/子目录/new.bin"), [1, 0, 255]).unwrap();
    fs::write(project.join("replace.bin"), [2, 0, 254]).unwrap();
    fs::write(project.join("unselected"), "skip").unwrap();
    fs::write(project.join(".env"), "fixture only").unwrap();
    fs::write(remote.join("replace.bin"), [3, 0, 253]).unwrap();
    fs::write(remote.join("remote-only"), "keep").unwrap();
    fs::set_permissions(
        remote.join("replace.bin"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let selected = || {
        let manifest = diff::scan(&project, &[]).unwrap();
        diff::select(
            &project,
            &manifest,
            &["assets/子目录/new.bin".into(), "replace.bin".into()],
        )
        .unwrap()
    };
    let plan = || async {
        service
            .review_diff(&space, remote.to_str().unwrap(), selected())
            .await
            .unwrap()
            .1
            .unwrap()
    };
    let cancel = AtomicBool::new(false);
    let old = plan().await;
    service.release(&space).await.unwrap();
    assert!(
        service
            .review_diff(&space, remote.to_str().unwrap(), selected())
            .await
            .unwrap()
            .1
            .is_none()
    );
    service.takeover(&space).await.unwrap();
    assert_eq!(
        service
            .apply_diff(old, project.clone(), &cancel)
            .await
            .err()
            .unwrap()
            .code,
        vibe_ssh_protocol::ErrorCode::StaleLease
    );

    let old = plan().await;
    fs::write(project.join("replace.bin"), [4, 0, 252]).unwrap();
    assert!(
        service
            .apply_diff(old, project.clone(), &cancel)
            .await
            .is_err()
    );
    assert!(!remote.join("assets").exists());

    let old = plan().await;
    fs::write(remote.join("replace.bin"), [5, 0, 251]).unwrap();
    assert!(
        service
            .apply_diff(old, project.clone(), &cancel)
            .await
            .is_err()
    );
    assert!(!remote.join("assets").exists());
    assert_eq!(fs::read(remote.join("replace.bin")).unwrap(), [5, 0, 251]);

    let approved = plan().await;
    assert_eq!(
        approved.entries().collect::<Vec<_>>(),
        [("assets/子目录/new.bin", false), ("replace.bin", true)]
    );
    let receipt = service
        .apply_diff(approved, project.clone(), &cancel)
        .await
        .unwrap();
    let receipt = serde_json::to_value(receipt).unwrap();
    assert!(receipt["stopped"].is_null(), "{receipt}");
    assert_eq!(receipt["applied"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read(remote.join("assets/子目录/new.bin")).unwrap(),
        [1, 0, 255]
    );
    assert_eq!(fs::read(remote.join("replace.bin")).unwrap(), [4, 0, 252]);
    assert_eq!(
        fs::metadata(remote.join("replace.bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        fs::read(receipt["applied"][1]["backup"].as_str().unwrap()).unwrap(),
        [5, 0, 251]
    );
    assert_eq!(fs::read(remote.join("remote-only")).unwrap(), b"keep");
    assert!(!remote.join("unselected").exists());
    assert!(!remote.join(".env").exists());
    assert!(
        service
            .review_diff(&space, remote.to_str().unwrap(), selected())
            .await
            .unwrap()
            .1
            .is_none()
    );
}

#[test]
fn bounded_metadata_and_selections_refuse_changed_links() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("folder")).unwrap();
    fs::write(root.path().join("folder/file"), "safe").unwrap();
    let metadata = diff::scan(root.path(), &[]).unwrap();
    fs::rename(root.path().join("folder"), root.path().join("original")).unwrap();
    symlink(root.path().join("original"), root.path().join("folder")).unwrap();
    assert!(diff::select(root.path(), &metadata, &["folder/file".into()]).is_err());
    for n in 0..1001 {
        fs::write(root.path().join(format!("item{n}")), []).unwrap();
    }
    let bounded = diff::scan(root.path(), &[]).unwrap();
    assert!(bounded.truncated);
    assert_eq!(bounded.entries.len(), 1000);
    assert!(diff::rules(&["**/secrets".into()]).is_err());
}
