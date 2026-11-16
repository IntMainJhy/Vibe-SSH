mod support;

use std::time::Duration;

use support::Fixture;
use vibe_ssh_core::{By, Error, ErrorCode};
use vibe_ssh_protocol::{JobStatus, Owner, Reason};

fn code<T>(result: Result<T, Error>) -> ErrorCode {
    result.err().expect("the operation should be refused").code
}

#[tokio::test]
async fn rejects_wrong_host_key_before_authentication() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let result = fixture
        .service
        .open("badkey", "goal", "a", Owner::Agent)
        .await;
    assert_eq!(code(result), ErrorCode::HostKeyMismatch);
    assert_eq!(fixture.count(&fixture.stats.authentications), 0);
    assert!(fixture.service.spaces(false).await.is_empty());
    Ok(())
}

#[tokio::test]
async fn host_key_probe_shows_the_fingerprint_without_authenticating() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let host = fixture.service.load_hosts()?.remove(0);
    assert_eq!(
        vibe_ssh_core::host_key(&host.host, host.port).await?,
        ("ssh-ed25519".to_owned(), host.host_key_sha256)
    );
    assert_eq!(fixture.count(&fixture.stats.authentications), 0);
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    assert_eq!(
        code(vibe_ssh_core::host_key("127.0.0.1", closed).await),
        ErrorCode::ConnectionFailed
    );
    Ok(())
}

#[tokio::test]
async fn refuses_a_rejected_identity_and_accepts_a_password_from_the_secret_source()
-> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let wrong = fixture
        .service
        .open("wronguser", "goal", "a", Owner::Agent)
        .await;
    assert_eq!(code(wrong), ErrorCode::AuthenticationFailed);
    let view = fixture
        .service
        .open("password", "goal", "a", Owner::Agent)
        .await?;
    let done = fixture
        .exec(view.control.lease.as_deref().unwrap(), "ok")
        .await?;
    assert_eq!(done.status, JobStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn agents_only_reach_hosts_the_user_enabled() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let visible = fixture.service.hosts(true)?;
    assert!(visible.iter().all(|host| host.id != "private"));
    assert!(
        fixture
            .service
            .hosts(false)?
            .iter()
            .any(|host| host.id == "private")
    );
    for name in ["private", "private server", "no-such-host"] {
        let result = fixture.service.open(name, "goal", "a", Owner::Agent).await;
        assert_eq!(code(result), ErrorCode::HostNotAuthorized);
    }
    assert_eq!(
        fixture.count(&fixture.stats.connections),
        0,
        "nothing was dialled"
    );
    fixture
        .service
        .open("private", "mine", "me", Owner::Human)
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_space_reuses_its_connection_and_drains_data_after_exit_status() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let failed = fixture.exec(&lease, "exit-7-with-tail").await?;
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.exit_code, Some(7));
    assert_eq!(failed.stdout.text, "start\ntail after status\n");
    assert_eq!(failed.stderr.text, "problem\n");
    assert_eq!(failed.stream_mode, "separate");
    assert_eq!(
        fixture.exec(&lease, "ok").await?.status,
        JobStatus::Succeeded
    );
    assert_eq!(fixture.count(&fixture.stats.connections), 1);
    assert_eq!(fixture.count(&fixture.stats.authentications), 1);
    assert_eq!(fixture.count(&fixture.stats.channels), 2);
    Ok(())
}

#[tokio::test]
async fn each_space_has_its_own_connection_and_losing_one_leaves_the_other_working()
-> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, agent) = fixture.agent().await?;
    let mine = fixture
        .service
        .open("fixture", "my terminal", "me", Owner::Human)
        .await?;
    assert_eq!(fixture.count(&fixture.stats.connections), 2);

    let lost = fixture.exec(&agent, "drop").await?;
    assert_eq!(
        lost.status,
        JobStatus::Unknown,
        "a lost connection is not a remote failure"
    );
    assert_eq!(lost.reason, Some(Reason::ConnectionLost));
    assert_eq!(lost.exit_code, None);
    let retry = fixture
        .service
        .run(By::Lease(&agent), Some("ok".into()), None, None)
        .await;
    assert_eq!(code(retry), ErrorCode::ConnectionFailed);

    let job = fixture
        .service
        .run(By::Human(&mine.space), Some("ok".into()), None, None)
        .await?;
    let done = fixture
        .service
        .job_settled(&job, 5_000, 8_192, false)
        .await?;
    assert_eq!(done.status, JobStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn missing_status_and_signal_are_not_success() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let missing = fixture.exec(&lease, "missing-status").await?;
    assert_eq!(missing.status, JobStatus::Unknown);
    assert_eq!(missing.reason, Some(Reason::MissingExitStatus));
    assert_eq!(missing.exit_code, None);
    let signaled = fixture.exec(&lease, "signaled").await?;
    assert_eq!(signaled.status, JobStatus::Failed);
    assert_eq!(signaled.reason, Some(Reason::ExitSignal));
    assert!(signaled.exit_signal.is_some());
    Ok(())
}

#[tokio::test]
async fn working_directory_is_quoted_for_the_remote_shell() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let by = By::Lease(&lease);
    let job = fixture
        .service
        .run(by, Some("ok".into()), Some("/srv/it's here".into()), None)
        .await?;
    let done = fixture.service.job_settled(&job, 5_000, 64, true).await?;
    assert_eq!(done.status, JobStatus::Succeeded);
    let relative = fixture
        .service
        .run(by, Some("ok".into()), Some("srv".into()), None)
        .await;
    assert_eq!(code(relative), ErrorCode::InvalidRequest);
    let shell_without_pty = fixture.service.run(by, None, None, None).await;
    assert_eq!(code(shell_without_pty), ErrorCode::InvalidRequest);
    assert_eq!(fixture.count(&fixture.stats.executions), 1);
    Ok(())
}

#[tokio::test]
async fn a_running_job_is_followed_with_cursors_and_an_expired_wait_is_not_a_failure()
-> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (space, lease) = fixture.agent().await?;
    let service = &fixture.service;
    let job = service
        .run(By::Lease(&lease), Some("interactive".into()), None, None)
        .await?;

    // A short initial wait returns the job still running: observation ended, the job did not.
    let first = service.job_settled(&job, 300, 8_192, true).await?;
    assert_eq!(first.status, JobStatus::Running);
    assert_eq!(first.stdout.text, "interactive ready\n");

    let idle = service.job_wait(&first.cursor, 8_192, 50, true).await?;
    assert_eq!(idle.status, JobStatus::Running);
    assert!(idle.stdout.text.is_empty() && !idle.stdout.truncated);
    assert_eq!(idle.cursor, first.cursor);

    service
        .input(By::Lease(&lease), &job, b"more\n", false)
        .await?;
    let next = service.job_wait(&idle.cursor, 8_192, 5_000, true).await?;
    assert_eq!(next.stdout.text, "more\n");

    // A takeover wakes a pending wait so the agent learns about it immediately.
    let waiting = {
        let (service, cursor) = (service.clone(), next.cursor.clone());
        tokio::spawn(async move { service.job_wait(&cursor, 8_192, 30_000, true).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    service.takeover(&space).await?;
    let woken = tokio::time::timeout(Duration::from_secs(2), waiting).await???;
    assert_eq!(woken.control.owner, Owner::Human);
    assert_eq!(woken.control.lease, None, "the user's lease is never shown");
    assert_eq!(
        woken.status,
        JobStatus::Running,
        "takeover does not kill the process"
    );

    let future = format!("{job}:999999:0");
    assert!(
        service
            .job_wait(&future, 64, 5_000, true)
            .await?
            .stdout
            .truncated
    );
    let foreign = service.job_wait("j1.zzzz:0:0", 64, 50, true).await;
    assert_eq!(code(foreign), ErrorCode::InstanceMismatch);
    Ok(())
}

#[tokio::test]
async fn large_output_is_bounded_and_read_incrementally_without_breaking_unicode()
-> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let job = fixture
        .service
        .run(By::Lease(&lease), Some("large".into()), None, None)
        .await?;
    let tail = fixture.service.job_settled(&job, 5_000, 31, true).await?;
    assert_eq!(tail.status, JobStatus::Succeeded);
    assert!(tail.stdout.truncated && !tail.stdout.invalid_utf8);
    assert!(tail.stdout.text.len() <= 31 && tail.stdout.text.ends_with("done"));
    assert!(tail.stdout.total_bytes > 30_000);

    let mut cursor = format!("{job}:0:0");
    let mut text = String::new();
    loop {
        let page = fixture
            .service
            .job_wait(&cursor, 1_000, 1_000, true)
            .await?;
        assert!(!page.stdout.invalid_utf8 && !page.stdout.truncated);
        assert!(page.stdout.text.len() <= 1_000);
        text.push_str(&page.stdout.text);
        cursor = page.cursor;
        if !page.more {
            break;
        }
    }
    assert_eq!(text, "部署🚀".repeat(3_200) + "done");
    Ok(())
}

#[tokio::test]
async fn rejected_pty_never_falls_through_to_running_the_command() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    fixture
        .stats
        .deny_pty
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = fixture
        .service
        .run(By::Lease(&lease), Some("ok".into()), None, Some((80, 24)))
        .await;
    assert_eq!(code(result), ErrorCode::ChannelRejected);
    assert_eq!(fixture.count(&fixture.stats.executions), 0);
    Ok(())
}

#[tokio::test]
async fn interrupt_is_a_request_and_the_remote_decides_the_outcome() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let service = &fixture.service;
    let by = By::Lease(&lease);

    let plain = service.run(by, Some("stall".into()), None, None).await?;
    service.interrupt(by, &plain).await?;
    let ended = service.job_settled(&plain, 5_000, 64, true).await?;
    assert!(ended.interrupt_requested);
    assert_eq!(ended.reason, Some(Reason::ExitSignal));
    assert_eq!(fixture.count(&fixture.stats.signals), 1);

    let terminal = service
        .run(by, Some("interactive".into()), None, Some((80, 24)))
        .await?;
    service.interrupt(by, &terminal).await?;
    let ended = service.job_settled(&terminal, 5_000, 64, true).await?;
    assert_eq!(
        (ended.status, ended.exit_code),
        (JobStatus::Failed, Some(130))
    );
    assert_eq!(ended.stream_mode, "merged");
    assert_eq!(fixture.received(), b"\x03");

    let late = service.interrupt(by, &terminal).await;
    assert_eq!(code(late), ErrorCode::InvalidRequest);
    Ok(())
}
