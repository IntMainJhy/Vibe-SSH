mod support;

use std::time::Duration;

use support::Fixture;
use vibe_ssh_core::{By, Error, ErrorCode};
use vibe_ssh_protocol::{JobStatus, Owner, Reason};

fn code<T>(result: Result<T, Error>) -> ErrorCode {
    result.err().expect("the operation should be refused").code
}

#[tokio::test]
async fn takeover_and_return_fence_every_agent_write_path() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let service = &fixture.service;
    let (space, agent) = fixture.agent().await?;
    let job = service
        .run(
            By::Lease(&agent),
            Some("interactive".into()),
            None,
            Some((100, 30)),
        )
        .await?;
    service
        .input(By::Lease(&agent), &job, b"agent\n", false)
        .await?;
    let human = By::Human(&space);
    assert_eq!(
        code(service.input(human, &job, b"forbidden\n", false).await),
        ErrorCode::ControlRevoked,
        "the user must take over before typing into an agent's space"
    );

    let taken = service.takeover(&space).await?;
    assert_eq!(taken.control.owner, Owner::Human);
    // These are late operations submitted only after the takeover was acknowledged.
    let late: Vec<_> = (0..16)
        .map(|_| {
            let (service, lease, job) = (service.clone(), agent.clone(), job.clone());
            tokio::spawn(async move {
                service
                    .input(By::Lease(&lease), &job, b"forbidden\n", false)
                    .await
            })
        })
        .collect();
    for task in late {
        assert_eq!(code(task.await?), ErrorCode::ControlRevoked);
    }
    let executions = fixture.count(&fixture.stats.executions);
    let by = By::Lease(&agent);
    assert_eq!(
        code(service.run(by, Some("ok".into()), None, None).await),
        ErrorCode::ControlRevoked,
        "a queued next step must not start"
    );
    assert_eq!(
        code(service.resize(by, &job, 90, 20).await),
        ErrorCode::ControlRevoked
    );
    assert_eq!(
        code(service.interrupt(by, &job).await),
        ErrorCode::ControlRevoked
    );
    assert_eq!(code(service.close(by).await), ErrorCode::ControlRevoked);
    assert_eq!(
        code(service.handoff(&agent, "again").await),
        ErrorCode::ControlRevoked
    );
    let local = fixture.home.path();
    assert_eq!(
        code(service.put(by, local, "/srv/app", &[]).await),
        ErrorCode::ControlRevoked
    );
    assert_eq!(fixture.count(&fixture.stats.executions), executions);
    assert_eq!(
        service.job(&job, 64, true).await?.status,
        JobStatus::Running
    );

    service.input(human, &job, b"human\n", false).await?;
    service.resize(human, &job, 120, 40).await?;

    let returned = service.release(&space).await?;
    let fresh = returned
        .control
        .lease
        .expect("the agent holds the space again");
    assert_ne!(fresh, agent);
    assert_eq!(
        code(
            service
                .input(By::Lease(&agent), &job, b"forbidden\n", false)
                .await
        ),
        ErrorCode::StaleLease,
        "the old lease never regains write access"
    );
    assert_eq!(
        code(service.input(human, &job, b"forbidden\n", false).await),
        ErrorCode::ControlRevoked
    );
    assert_eq!(
        service.space(&space, true).await?.control.lease.as_deref(),
        Some(&*fresh)
    );
    service
        .input(By::Lease(&fresh), &job, b"finish\n", false)
        .await?;
    let done = service.job_settled(&job, 5_000, 8_192, true).await?;
    assert_eq!(done.status, JobStatus::Succeeded);
    assert!(done.stdout.text.contains("agent\nhuman\nfinish\n"));
    fixture.expect_received(b"agent\nhuman\nfinish\n").await;
    assert_eq!(fixture.count(&fixture.stats.resizes), 1);
    Ok(())
}

#[tokio::test]
async fn handoff_gives_the_user_control_with_a_reason_until_they_return_it() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let service = &fixture.service;
    let (space, agent) = fixture.agent().await?;
    let job = service
        .run(
            By::Lease(&agent),
            Some("interactive".into()),
            None,
            Some((80, 24)),
        )
        .await?;
    let handed = service.handoff(&agent, "sudo needs your password").await?;
    assert_eq!(handed.control.owner, Owner::Human);
    assert_eq!(
        handed.control.note.as_deref(),
        Some("sudo needs your password")
    );
    assert_eq!(handed.control.lease, None);
    assert_eq!(
        code(
            service
                .input(By::Lease(&agent), &job, b"guess\n", false)
                .await
        ),
        ErrorCode::ControlRevoked
    );
    service
        .input(By::Human(&space), &job, b"secret\n", false)
        .await?;
    let returned = service.release(&space).await?;
    assert_eq!(returned.control.note, None);
    let fresh = returned.control.lease.unwrap();
    service
        .input(By::Lease(&fresh), &job, b"finish\n", false)
        .await?;
    fixture.expect_received(b"secret\nfinish\n").await;
    Ok(())
}

#[tokio::test]
async fn a_lease_only_works_in_its_own_space() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let service = &fixture.service;
    let (_, first) = fixture.agent().await?;
    let (_, second) = fixture.agent().await?;
    let job = service
        .run(By::Lease(&second), Some("interactive".into()), None, None)
        .await?;
    assert_eq!(
        code(
            service
                .input(By::Lease(&first), &job, b"forbidden\n", false)
                .await
        ),
        ErrorCode::InvalidRequest
    );
    let forged = format!("{}.000000000000", second.rsplit_once('.').unwrap().0);
    assert_eq!(
        code(
            service
                .input(By::Lease(&forged), &job, b"forbidden\n", false)
                .await
        ),
        ErrorCode::StaleLease
    );
    let oversized = vec![b'x'; 32_769];
    assert_eq!(
        code(
            service
                .input(By::Lease(&second), &job, &oversized, false)
                .await
        ),
        ErrorCode::InvalidRequest
    );
    service
        .input(By::Lease(&second), &job, b"finish\n", false)
        .await?;
    fixture.expect_received(b"finish\n").await;
    Ok(())
}

#[tokio::test]
async fn the_users_own_spaces_are_invisible_and_closed_to_agents() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let service = &fixture.service;
    let mine = service
        .open("fixture", "my terminal", "me", Owner::Human)
        .await?;
    assert_eq!(mine.control.lease, None);
    let shell = service
        .run(By::Human(&mine.space), None, None, Some((80, 24)))
        .await?;
    service
        .input(By::Human(&mine.space), &shell, b"typed by me\n", false)
        .await?;

    assert!(service.spaces(true).await.is_empty());
    assert_eq!(
        code(service.space(&mine.space, true).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(service.job(&shell, 64, true).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(
            service
                .job_wait(&format!("{shell}:0:0"), 64, 50, true)
                .await
        ),
        ErrorCode::NotFound
    );
    let forged = format!("{}.000000000000", mine.space);
    let by = By::Lease(&forged);
    assert_eq!(
        code(service.input(by, &shell, b"x\n", false).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(service.run(by, Some("ok".into()), None, None).await),
        ErrorCode::NotFound
    );
    assert_eq!(code(service.close(by).await), ErrorCode::NotFound);
    assert_eq!(
        code(service.release(&mine.space).await),
        ErrorCode::InvalidRequest,
        "the user's own space can never be handed to an agent"
    );
    fixture.expect_received(b"typed by me\n").await;

    service.close(By::Human(&mine.space)).await?;
    assert_eq!(
        code(service.job(&shell, 64, false).await),
        ErrorCode::NotFound
    );
    Ok(())
}

#[tokio::test]
async fn upload_streams_a_directory_without_excluded_paths() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_, lease) = fixture.agent().await?;
    let project = tempfile::tempdir()?;
    std::fs::create_dir_all(project.path().join("node_modules/dep"))?;
    std::fs::create_dir_all(project.path().join("页面"))?;
    std::fs::write(project.path().join("node_modules/dep/index.js"), "skip me")?;
    std::fs::write(project.path().join(".env"), "SECRET=1")?;
    std::fs::write(project.path().join("页面/index.html"), "<h1>你好 🚀</h1>")?;
    let excludes = ["node_modules".to_owned(), ".env".to_owned()];
    let job = fixture
        .service
        .put(
            By::Lease(&lease),
            project.path(),
            "/var/www/it's",
            &excludes,
        )
        .await?;
    let done = fixture
        .service
        .job_settled(&job, 10_000, 8_192, true)
        .await?;
    assert_eq!(done.status, JobStatus::Succeeded, "{}", done.stderr.text);
    assert_eq!(
        fixture.stats.commands.lock().unwrap().last().unwrap(),
        "mkdir -p '/var/www/it'\\''s' && tar -xof - -C '/var/www/it'\\''s'"
    );

    // The fixture kept the stream; unpack it the way the remote tar would.
    let remote = tempfile::tempdir()?;
    std::fs::write(remote.path().join("upload.tar"), fixture.received())?;
    let unpacked = std::process::Command::new("/usr/bin/tar")
        .args(["-xf", "upload.tar"])
        .current_dir(remote.path())
        .status()?;
    assert!(unpacked.success());
    assert_eq!(
        std::fs::read_to_string(remote.path().join("页面/index.html"))?,
        "<h1>你好 🚀</h1>"
    );
    assert!(!remote.path().join("node_modules").exists());
    assert!(!remote.path().join(".env").exists());

    let missing = project.path().join("nope");
    let result = fixture
        .service
        .put(By::Lease(&lease), &missing, "/srv", &[])
        .await;
    assert_eq!(code(result), ErrorCode::NotFound);
    Ok(())
}

#[tokio::test]
async fn takeover_stops_an_upload_in_progress_and_the_job_says_so() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (space, lease) = fixture.agent().await?;
    let project = tempfile::tempdir()?;
    let size = 64 * 1024 * 1024;
    std::fs::File::create(project.path().join("big.bin"))?.set_len(size)?;
    let job = fixture
        .service
        .put(By::Lease(&lease), project.path(), "/srv/app", &[])
        .await?;
    fixture.service.takeover(&space).await?;
    let ended = fixture
        .service
        .job_settled(&job, 10_000, 8_192, true)
        .await?;
    assert_eq!(
        ended.status,
        JobStatus::Failed,
        "a partial upload is never a success"
    );
    assert_eq!(ended.reason, Some(Reason::UploadIncomplete));
    assert_eq!(ended.exit_code, None, "no remote exit status is invented");
    assert!(
        ended
            .stderr
            .text
            .contains("upload stopped before completion")
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!((fixture.received().len() as u64) < size);
    Ok(())
}
