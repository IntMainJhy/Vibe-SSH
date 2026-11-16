//! Deterministic SSH/SFTP protocol fixture. It never executes received commands.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use russh::{
    Channel, ChannelId, Pty,
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey},
    server::{self, Auth, Msg, Server as _, Session, VerifiedPublicKey},
};
use russh_sftp::protocol::{Data, FileAttributes, Handle, OpenFlags, Status, StatusCode};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use vibe_ssh_core::ConnectionConfig;

#[derive(Default)]
pub struct Stats {
    pub connections: AtomicUsize,
    pub authentications: AtomicUsize,
    pub channels: AtomicUsize,
    pub ptys: AtomicUsize,
    pub resizes: AtomicUsize,
    pub executions: AtomicUsize,
    pub deny_pty: AtomicBool,
    pub received: Mutex<Vec<u8>>,
}

pub struct Fixture {
    pub config: ConnectionConfig,
    pub stats: Arc<Stats>,
    shutdown: server::RunningServerHandle,
    task: JoinHandle<()>,
    _directory: TempDir,
}

impl Fixture {
    pub async fn start() -> anyhow::Result<Self> {
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        let directory = tempfile::tempdir()?;
        let identity_file = directory.path().join("identity");
        identity.write_openssh_file(&identity_file, Default::default())?;
        let stats = Arc::new(Stats::default());
        let mut server = TestServer {
            key: identity.public_key().clone(),
            stats: stats.clone(),
        };
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let config = ConnectionConfig {
            host: "127.0.0.1".into(),
            port,
            username: "fixture".into(),
            identity_file,
            host_key_sha256: host_key
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        };
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
            config,
            stats,
            shutdown,
            task,
            _directory: directory,
        })
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
        }
    }
}

struct TestSession {
    key: PublicKey,
    stats: Arc<Stats>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl TestSession {
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

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stats.executions.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel)?;
        match command {
            b"ok" => {
                session.data(channel, b"ready\n".to_vec())?;
                Self::finish(session, channel, Some(0))?;
            }
            b"exit-7-with-tail" => {
                session.data(channel, b"start\n".to_vec())?;
                session.extended_data(channel, 1, b"problem\n".to_vec())?;
                session.exit_status_request(channel, 7)?;
                session.data(channel, b"tail after status\n".to_vec())?;
                Self::finish(session, channel, None)?;
            }
            b"missing-status" => Self::finish(session, channel, None)?,
            b"signaled" => {
                session.exit_signal_request(
                    channel,
                    russh::Sig::TERM,
                    false,
                    "test signal",
                    "en",
                )?;
                Self::finish(session, channel, None)?;
            }
            b"large" => {
                for _ in 0..32 {
                    session.data(channel, "部署🚀".repeat(100).into_bytes())?;
                }
                session.data(channel, b"done".to_vec())?;
                Self::finish(session, channel, Some(0))?;
            }
            b"interactive" => {
                session.data(channel, b"interactive ready\n".to_vec())?;
            }
            b"stall" => {}
            _ => Self::finish(session, channel, Some(127))?,
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Subsystem bytes belong to the SFTP stream, never the terminal echo path.
        if !self.channels.contains_key(&channel) {
            return Ok(());
        }
        self.stats.received.lock().await.extend_from_slice(data);
        session.data(channel, data.to_vec())?;
        if data == b"finish\n" {
            Self::finish(session, channel, Some(0))?;
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            return session.channel_failure(channel);
        }
        session.channel_success(channel)?;
        if let Some(channel) = self.channels.remove(&channel) {
            tokio::spawn(russh_sftp::server::run(
                channel.into_stream(),
                MemorySftp::default(),
            ));
        }
        Ok(())
    }
}

/// A virtual fixture file, not a production filesystem adapter.
#[derive(Default)]
struct MemorySftp {
    contents: Vec<u8>,
}

impl russh_sftp::server::Handler for MemorySftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if filename != "fixture.txt" {
            return Err(StatusCode::PermissionDenied);
        }
        if flags.contains(OpenFlags::TRUNCATE) {
            self.contents.clear();
        }
        Ok(Handle {
            id,
            handle: filename,
        })
    }

    async fn write(
        &mut self,
        id: u32,
        _: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let start = usize::try_from(offset).map_err(|_| StatusCode::Failure)?;
        let end = start.checked_add(data.len()).ok_or(StatusCode::Failure)?;
        if end > 1_048_576 {
            return Err(StatusCode::Failure);
        }
        self.contents.resize(self.contents.len().max(end), 0);
        self.contents[start..end].copy_from_slice(&data);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: String::new(),
            language_tag: "en".into(),
        })
    }

    async fn read(
        &mut self,
        id: u32,
        _: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let start = usize::try_from(offset).map_err(|_| StatusCode::Failure)?;
        if start >= self.contents.len() {
            return Err(StatusCode::Eof);
        }
        let end = start.saturating_add(len as usize).min(self.contents.len());
        Ok(Data {
            id,
            data: self.contents[start..end].to_vec(),
        })
    }

    async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: String::new(),
            language_tag: "en".into(),
        })
    }
}
