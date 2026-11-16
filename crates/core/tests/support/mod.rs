//! Deterministic SSH protocol fixture. It never executes received commands.
#![allow(dead_code)] // each test binary uses a different part of the fixture

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use russh::{
    Channel, ChannelId, Pty, Sig,
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey},
    server::{self, Auth, Msg, Server as _, Session, VerifiedPublicKey},
};
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle};
use vibe_ssh_core::{By, Host, Service};
use vibe_ssh_protocol::{JobView, Owner};

pub const PASSWORD: &str = "fixture-password";

#[derive(Default)]
pub struct Stats {
    pub connections: AtomicUsize,
    pub authentications: AtomicUsize,
    pub channels: AtomicUsize,
    pub ptys: AtomicUsize,
    pub resizes: AtomicUsize,
    pub executions: AtomicUsize,
    pub signals: AtomicUsize,
    pub deny_pty: AtomicBool,
    pub received: Mutex<Vec<u8>>,
    pub commands: Mutex<Vec<String>>,
}

pub struct Fixture {
    pub service: Arc<Service>,
    pub stats: Arc<Stats>,
    pub home: TempDir,
    shutdown: server::RunningServerHandle,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> anyhow::Result<Self> {
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        let home = tempfile::tempdir()?;
        let identity_file = home.path().join("identity");
        identity.write_openssh_file(&identity_file, Default::default())?;
        let stats = Arc::new(Stats::default());
        let mut server = TestServer {
            key: identity.public_key().clone(),
            stats: stats.clone(),
        };
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let host = |id: &str| Host {
            id: id.into(),
            name: format!("{id} server"),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: "fixture".into(),
            identity_file: Some(identity_file.clone()),
            host_key_sha256: host_key
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
            agent_access: true,
        };
        let service = Service::new(
            home.path().to_owned(),
            Box::new(|host| (host.id == "password").then(|| PASSWORD.to_owned())),
        );
        service.save_hosts(&[
            host("fixture"),
            Host {
                agent_access: false,
                ..host("private")
            },
            Host {
                identity_file: None,
                ..host("password")
            },
            Host {
                host_key_sha256: "SHA256:incorrect".into(),
                ..host("badkey")
            },
            Host {
                username: "not-the-fixture-user".into(),
                ..host("wronguser")
            },
        ])?;
        let server_config = Arc::new(server::Config {
            keys: vec![host_key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        // Obtain a shutdown handle before moving the borrowed server/listener into a task.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let running = server.run_on_socket(server_config, &listener);
            let _ = ready_tx.send(running.handle());
            let _ = running.await;
        });
        let shutdown = ready_rx.await?;
        Ok(Self {
            service,
            stats,
            home,
            shutdown,
            task,
        })
    }

    /// Opens an agent space on the fixture host. Returns `(space, lease)`.
    pub async fn agent(&self) -> anyhow::Result<(String, String)> {
        let view = self
            .service
            .open("fixture", "test goal", "test-agent", Owner::Agent)
            .await?;
        Ok((
            view.space,
            view.control.lease.expect("agent owns a new space"),
        ))
    }

    /// Runs a command as the agent and waits for it to finish.
    pub async fn exec(&self, lease: &str, command: &str) -> anyhow::Result<JobView> {
        let job = self
            .service
            .run(By::Lease(lease), Some(command.into()), None, None)
            .await?;
        Ok(self.service.job_settled(&job, 5_000, 8_192, true).await?)
    }

    pub fn count(&self, counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }

    /// Input is acknowledged once queued on the transport, so the server sees it slightly later.
    pub async fn expect_received(&self, expected: &[u8]) {
        for _ in 0..200 {
            if self.received() == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            String::from_utf8_lossy(&self.received()),
            String::from_utf8_lossy(expected)
        );
    }

    pub fn received(&self) -> Vec<u8> {
        self.stats.received.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.shutdown("fixture cleanup".into());
        self.task.abort();
    }
}

struct TestServer {
    key: PublicKey,
    stats: Arc<Stats>,
}

impl server::Server for TestServer {
    type Handler = TestSession;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self::Handler {
        self.stats.connections.fetch_add(1, Ordering::SeqCst);
        TestSession {
            key: self.key.clone(),
            stats: self.stats.clone(),
            channels: HashMap::new(),
            uploads: HashSet::new(),
        }
    }
}

struct TestSession {
    key: PublicKey,
    stats: Arc<Stats>,
    channels: HashMap<ChannelId, Channel<Msg>>,
    /// Channels whose stdin is a file stream rather than terminal input.
    uploads: HashSet<ChannelId>,
}

fn finish(
    session: &mut Session,
    channel: ChannelId,
    code: Option<u32>,
) -> Result<(), russh::Error> {
    if let Some(code) = code {
        session.exit_status_request(channel, code)?;
    }
    session.eof(channel)?;
    session.close(channel)
}

impl server::Handler for TestSession {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        username: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        self.stats.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(if username == "fixture" && key.public_key == &self.key {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<Auth, Self::Error> {
        self.stats.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(if username == "fixture" && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.channels.fetch_add(1, Ordering::SeqCst);
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.ptys.fetch_add(1, Ordering::SeqCst);
        if self.stats.deny_pty.load(Ordering::SeqCst) {
            session.channel_failure(channel)
        } else {
            session.channel_success(channel)
        }
    }

    async fn window_change_request(
        &mut self,
        _: ChannelId,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.resizes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.executions.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel)?;
        session.data(channel, b"shell ready\n".to_vec())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.executions.fetch_add(1, Ordering::SeqCst);
        let command = String::from_utf8_lossy(command).into_owned();
        self.stats.commands.lock().unwrap().push(command.clone());
        session.channel_success(channel)?;
        match command.as_str() {
            "ok" | "cd '/srv/it'\\''s here' && ok" => {
                session.data(channel, b"ready\n".to_vec())?;
                finish(session, channel, Some(0))?;
            }
            "exit-7-with-tail" => {
                session.data(channel, b"start\n".to_vec())?;
                session.extended_data(channel, 1, b"problem\n".to_vec())?;
                session.exit_status_request(channel, 7)?;
                session.data(channel, b"tail after status\n".to_vec())?;
                finish(session, channel, None)?;
            }
            "missing-status" => finish(session, channel, None)?,
            "signaled" => {
                session.exit_signal_request(channel, Sig::TERM, false, "test signal", "en")?;
                finish(session, channel, None)?;
            }
            "large" => {
                for _ in 0..32 {
                    session.data(channel, "部署🚀".repeat(100).into_bytes())?;
                }
                session.data(channel, b"done".to_vec())?;
                finish(session, channel, Some(0))?;
            }
            "interactive" => session.data(channel, b"interactive ready\n".to_vec())?,
            "stall" => {}
            // The transport dies under a running job.
            "drop" => return Err(russh::Error::Disconnect),
            upload if upload.starts_with("mkdir -p ") => {
                self.uploads.insert(channel);
            }
            _ => finish(session, channel, Some(127))?,
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.received.lock().unwrap().extend_from_slice(data);
        if self.uploads.contains(&channel) {
            return Ok(());
        }
        session.data(channel, data.to_vec())?;
        match data {
            b"finish\n" | b"exit\n" => finish(session, channel, Some(0)),
            b"\x03" => finish(session, channel, Some(130)),
            _ => Ok(()),
        }
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.uploads.remove(&channel) {
            finish(session, channel, Some(0))?;
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.signals.fetch_add(1, Ordering::SeqCst);
        session.exit_signal_request(channel, signal, false, "interrupted", "en")?;
        finish(session, channel, None)
    }
}
