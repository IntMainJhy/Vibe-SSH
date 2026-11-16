mod support;

use std::{sync::atomic::Ordering, time::Duration};

use support::Fixture;
use vibe_ssh_core::{Error, SshConnection};
use vibe_ssh_protocol::{CompletionReason, ExecutionRequest, InputOwner, JobStatus};

fn request(command: &str) -> ExecutionRequest {
    ExecutionRequest {
        command: command.into(),
        cwd: None,
        pty: false,
        timeout_ms: 5_000,
        output_limit: 8192,
    }
}

#[tokio::test]
async fn rejects_wrong_host_key_before_authentication() -> anyhow::Result<()> {
    let mut fixture = Fixture::start().await?;
    fixture.config.host_key_sha256 = "SHA256:incorrect".into();
    let result = SshConnection::connect(&fixture.config).await;
    assert!(matches!(result, Err(Error::HostKeyMismatch)));
    assert_eq!(fixture.stats.authentications.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn refuses_unknown_identity_instead_of_falling_back() -> anyhow::Result<()> {
    let mut fixture = Fixture::start().await?;
    fixture.config.username = "not-the-fixture-user".into();
    assert!(matches!(
        SshConnection::connect(&fixture.config).await,
        Err(Error::AuthenticationFailed)
    ));
    Ok(())
}

#[tokio::test]
async fn reuses_authenticated_connection_and_drains_data_after_exit_status() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let failed = connection.run(request("exit-7-with-tail")).await?;
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.exit_code, Some(7));
    assert_eq!(failed.stdout.text, "start\ntail after status\n");
    assert_eq!(failed.stderr.text, "problem\n");
    let success = connection.run(request("ok")).await?;
    assert_eq!(success.status, JobStatus::Succeeded);
    assert_eq!(fixture.stats.connections.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.stats.authentications.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.stats.channels.load(Ordering::SeqCst), 2);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn missing_status_and_signal_are_not_success() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let missing = connection.run(request("missing-status")).await?;
    assert_eq!(missing.status, JobStatus::Unknown);
    assert_eq!(missing.exit_code, None);
    assert_eq!(
        missing.completion_reason,
        CompletionReason::MissingExitStatus
    );
    let signaled = connection.run(request("signaled")).await?;
    assert_eq!(signaled.status, JobStatus::Failed);
    assert!(signaled.exit_signal.is_some());
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn timeout_preserves_unknown_remote_outcome() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let mut spec = request("stall");
    spec.timeout_ms = 50;
    let result = tokio::time::timeout(Duration::from_secs(2), connection.run(spec)).await??;
    assert_eq!(result.status, JobStatus::Unknown);
    assert_eq!(
        result.completion_reason,
        CompletionReason::ObservationTimeout
    );
    assert_eq!(result.exit_code, None);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn bounds_real_ssh_output_without_breaking_unicode() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let mut spec = request("large");
    spec.output_limit = 31;
    let result = connection.run(spec).await?;
    assert!(result.stdout.truncated);
    assert!(result.stdout.retained_bytes <= 31);
    assert!(result.stdout.total_bytes > 30_000);
    assert!(result.stdout.text.ends_with("done"));
    assert!(!result.stdout.invalid_utf8);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn takeover_and_return_fence_actual_pty_transport_writes() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let mut spec = request("interactive");
    spec.pty = true;
    let running = connection.start(spec).await?;
    let input = running.input.clone();
    let agent = running.initial_lease.clone();
    input.write(&agent, b"agent\n").await?;
    let human = input.transfer(InputOwner::Human).await?;
    // These are late operations submitted only after takeover acknowledged.
    let late: Vec<_> = (0..16)
        .map(|_| {
            let input = input.clone();
            let lease = agent.clone();
            tokio::spawn(async move { input.write(&lease, b"forbidden\n").await })
        })
        .collect();
    for task in late {
        assert!(matches!(task.await?, Err(Error::ControlRevoked)));
    }
    input.write(&human, b"human\n").await?;
    input.resize(&human, 120, 40).await?;
    let returned = input.transfer(InputOwner::Agent).await?;
    assert!(matches!(
        input.write(&human, b"forbidden\n").await,
        Err(Error::ControlRevoked)
    ));
    assert!(matches!(
        input.write(&agent, b"forbidden\n").await,
        Err(Error::ControlRevoked)
    ));
    assert!(returned.epoch() > human.epoch());
    input.write(&returned, b"finish\n").await?;
    let result = running.collect().await?;
    assert_eq!(result.status, JobStatus::Succeeded);
    assert!(result.output_merged);
    assert!(result.stdout.text.contains("agent\nhuman\nfinish\n"));
    assert_eq!(
        &*fixture.stats.received.lock().await,
        b"agent\nhuman\nfinish\n"
    );
    assert_eq!(fixture.stats.ptys.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.stats.resizes.load(Ordering::SeqCst), 1);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn lease_cannot_be_reused_on_another_channel() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let first = connection.start(request("interactive")).await?;
    let second = connection.start(request("interactive")).await?;
    assert!(matches!(
        second
            .input
            .write(&first.initial_lease, b"forbidden\n")
            .await,
        Err(Error::ControlRevoked)
    ));
    first.input.write(&first.initial_lease, b"finish\n").await?;
    second
        .input
        .write(&second.initial_lease, b"finish\n")
        .await?;
    assert_eq!(first.collect().await?.status, JobStatus::Succeeded);
    assert_eq!(second.collect().await?.status, JobStatus::Succeeded);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn transfers_file_bytes_over_sftp_on_the_existing_connection() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let sftp = connection.sftp().await?;
    let expected = "Vibe-SSH 文件传输 🚀".as_bytes();
    sftp.write("fixture.txt", expected).await?;
    assert_eq!(sftp.read("fixture.txt").await?, expected);
    sftp.close().await?;
    assert_eq!(
        connection.run(request("ok")).await?.status,
        JobStatus::Succeeded
    );
    assert_eq!(fixture.stats.connections.load(Ordering::SeqCst), 1);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn rejected_pty_never_falls_through_to_running_the_command() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    fixture.stats.deny_pty.store(true, Ordering::SeqCst);
    let connection = SshConnection::connect(&fixture.config).await?;
    let mut spec = request("ok");
    spec.pty = true;
    assert!(matches!(
        connection.start(spec).await,
        Err(Error::ChannelRejected)
    ));
    assert_eq!(fixture.stats.executions.load(Ordering::SeqCst), 0);
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn oversized_input_is_rejected_before_transport_and_takeover_remains_available()
-> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let connection = SshConnection::connect(&fixture.config).await?;
    let running = connection.start(request("interactive")).await?;
    assert!(matches!(
        running
            .input
            .write(&running.initial_lease, &vec![b'x'; 32_769])
            .await,
        Err(Error::InvalidRequest(_))
    ));
    let human = running.input.transfer(InputOwner::Human).await?;
    running.input.write(&human, b"finish\n").await?;
    assert_eq!(running.collect().await?.status, JobStatus::Succeeded);
    assert_eq!(&*fixture.stats.received.lock().await, b"finish\n");
    connection.disconnect().await?;
    Ok(())
}
