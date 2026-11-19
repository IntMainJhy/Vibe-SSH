use super::*;
use serde::Serialize;
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Serialize)]
pub struct LocalForward {
    pub id: String,
    pub local_port: u16,
    #[serde(flatten)]
    mode: ForwardMode,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ForwardMode {
    Local { target: String, target_port: u16 },
    Dynamic,
    Remote { remote_port: u16 },
}

pub(super) struct ForwardTask {
    rule: LocalForward,
    task: Option<JoinHandle<()>>,
}

pub(super) type ForwardSet = BTreeMap<String, ForwardTask>;

impl Drop for ForwardTask {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Service {
    fn personal_space(&self, id: &str) -> Result<Arc<Space>> {
        let space = self.find_space(id, false)?;
        if space.origin != Owner::Human {
            return err(ErrorCode::InvalidRequest, "端口转发只可在个人会话中使用。");
        }
        Ok(space)
    }

    pub async fn list_forwards(&self, id: &str) -> Result<Vec<LocalForward>> {
        let space = self.personal_space(id)?;
        let mut forwards = space.forwards.lock().await;
        forwards.retain(|_, forward| {
            !space.conn.is_closed() && !forward.task.as_ref().is_some_and(JoinHandle::is_finished)
        });
        Ok(forwards
            .values()
            .map(|forward| forward.rule.clone())
            .collect())
    }

    pub async fn start_forward(
        &self,
        id: &str,
        local_port: u16,
        target: String,
        target_port: u16,
    ) -> Result<LocalForward> {
        if !valid_target(&target, target_port) {
            return err(ErrorCode::InvalidRequest, "请填写有效的目标主机与端口。");
        }
        self.listen_forward(
            id,
            local_port,
            ForwardMode::Local {
                target,
                target_port,
            },
        )
        .await
    }

    pub async fn start_dynamic_forward(&self, id: &str, local_port: u16) -> Result<LocalForward> {
        self.listen_forward(id, local_port, ForwardMode::Dynamic)
            .await
    }

    pub async fn start_remote_forward(
        &self,
        id: &str,
        remote_port: u16,
        local_port: u16,
    ) -> Result<LocalForward> {
        if local_port == 0 {
            return err(ErrorCode::InvalidRequest, "本地目标端口必须大于 0。");
        }
        let space = self.personal_space(id)?;
        let mut forwards = self.forward_slots(id, &space).await?;
        let remote_port = space.conn.listen_remote(remote_port, local_port).await?;
        let rule = LocalForward {
            id: token(16),
            local_port,
            mode: ForwardMode::Remote { remote_port },
        };
        forwards.insert(
            rule.id.clone(),
            ForwardTask {
                rule: rule.clone(),
                task: None,
            },
        );
        Ok(rule)
    }

    async fn forward_slots<'a>(
        &self,
        id: &str,
        space: &'a Space,
    ) -> Result<tokio::sync::MutexGuard<'a, ForwardSet>> {
        let mut forwards = space.forwards.lock().await;
        forwards.retain(|_, forward| !forward.task.as_ref().is_some_and(JoinHandle::is_finished));
        if forwards.len() >= 8 {
            return err(ErrorCode::ResourceLimit, "每个个人会话最多 8 条转发。");
        }
        if space.conn.is_closed() || self.find_space(id, false).is_err() {
            return err(ErrorCode::ConnectionFailed, "会话已断开，请重新连接。");
        }
        Ok(forwards)
    }

    async fn listen_forward(
        &self,
        id: &str,
        local_port: u16,
        mode: ForwardMode,
    ) -> Result<LocalForward> {
        let space = self.personal_space(id)?;
        let mut forwards = self.forward_slots(id, &space).await?;
        let listener = TcpListener::bind(("127.0.0.1", local_port))
            .await
            .map_err(|error| {
                Error::new(
                    ErrorCode::InvalidRequest,
                    format!("无法监听本地端口：{error}"),
                )
            })?;
        let rule = LocalForward {
            id: token(16),
            local_port: listener.local_addr()?.port(),
            mode: mode.clone(),
        };
        let owner = Arc::downgrade(&space);
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            let mut heartbeat = tokio::time::interval(Duration::from_millis(500));
            loop {
                let Some(space) = owner.upgrade() else {
                    break;
                };
                if space.conn.is_closed() {
                    break;
                }
                drop(space);
                tokio::select! {
                    accepted = listener.accept(), if clients.len() < 16 => {
                        let Ok((local, _)) = accepted else { break; };
                        let Some(space) = owner.upgrade() else { break; };
                        let mode = mode.clone();
                        clients.spawn(async move {
                            let _ = forward_client(&space.conn, local, mode).await;
                        });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                    _ = heartbeat.tick() => {}
                }
            }
        });
        forwards.insert(
            rule.id.clone(),
            ForwardTask {
                rule: rule.clone(),
                task: Some(task),
            },
        );
        Ok(rule)
    }

    pub async fn stop_forward(&self, id: &str, rule: &str) -> Result<()> {
        let space = self.personal_space(id)?;
        let mut forwards = space.forwards.lock().await;
        if let Some(ForwardMode::Remote { remote_port }) =
            forwards.get(rule).map(|task| &task.rule.mode)
        {
            space.conn.stop_remote(*remote_port).await?;
        }
        if let Some(task) = forwards
            .remove(rule)
            .and_then(|mut forward| forward.task.take())
        {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }

    pub(super) async fn clear_forwards(&self, space: &Space) {
        let tasks = std::mem::take(&mut *space.forwards.lock().await);
        for (_, mut forward) in tasks {
            if let Some(task) = forward.task.take() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

fn valid_target(target: &str, port: u16) -> bool {
    port != 0
        && !target.is_empty()
        && target.len() <= 253
        && target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-:".contains(&byte))
}

async fn forward_client(conn: &Connection, mut local: TcpStream, mode: ForwardMode) -> Result<()> {
    let dynamic = matches!(mode, ForwardMode::Dynamic);
    let (host, port) = match mode {
        ForwardMode::Local {
            target,
            target_port,
        } => (target, target_port),
        ForwardMode::Dynamic => {
            let Ok(Ok(Some(target))) =
                tokio::time::timeout(Duration::from_secs(5), socks_target(&mut local)).await
            else {
                return Ok(());
            };
            target
        }
        ForwardMode::Remote { .. } => return Ok(()),
    };
    let channel = conn.tcp_channel(&host, port).await;
    if dynamic {
        socks_reply(&mut local, if channel.is_ok() { 0 } else { 1 }).await?;
    }
    let mut tunnel = channel?.into_stream();
    copy_bidirectional(&mut local, &mut tunnel).await?;
    Ok(())
}

async fn socks_target(stream: &mut TcpStream) -> std::io::Result<Option<(String, u16)>> {
    let version = stream.read_u8().await?;
    let length = stream.read_u8().await? as usize;
    if version != 5 || length == 0 {
        return Ok(None);
    }
    let mut methods = [0; 255];
    stream.read_exact(&mut methods[..length]).await?;
    if !methods[..length].contains(&0) {
        stream.write_all(&[5, 255]).await?;
        return Ok(None);
    }
    stream.write_all(&[5, 0]).await?;

    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[2] != 0 {
        socks_reply(stream, 1).await?;
        return Ok(None);
    }
    if header[1] != 1 {
        socks_reply(stream, 7).await?;
        return Ok(None);
    }
    let host = match header[3] {
        1 => {
            let mut address = [0; 4];
            stream.read_exact(&mut address).await?;
            Ipv4Addr::from(address).to_string()
        }
        3 => {
            let size = stream.read_u8().await? as usize;
            let mut address = vec![0; size];
            stream.read_exact(&mut address).await?;
            String::from_utf8(address).unwrap_or_default()
        }
        4 => {
            let mut address = [0; 16];
            stream.read_exact(&mut address).await?;
            Ipv6Addr::from(address).to_string()
        }
        _ => {
            socks_reply(stream, 8).await?;
            return Ok(None);
        }
    };
    let port = stream.read_u16().await?;
    if !valid_target(&host, port) {
        socks_reply(stream, 8).await?;
        return Ok(None);
    }
    Ok(Some((host, port)))
}

async fn socks_reply(stream: &mut TcpStream, code: u8) -> std::io::Result<()> {
    stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}
