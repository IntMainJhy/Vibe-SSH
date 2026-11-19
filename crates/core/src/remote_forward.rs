use super::*;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    net::IpAddr,
};
use tokio::{io::copy_bidirectional, net::TcpStream, task::JoinSet};

const LOOPBACK: &str = "127.0.0.1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct RemoteRoutes(Mutex<BTreeMap<u16, RemoteRoute>>);

struct RemoteRoute {
    local_port: u16,
    clients: JoinSet<()>,
}

impl RemoteRoutes {
    pub(super) fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    pub(super) fn relay(
        &self,
        reported_listener: &str,
        remote_port: u32,
        reported_peer: &str,
        channel: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
    ) {
        if reported_listener != LOOPBACK
            || !reported_peer
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.to_canonical().is_loopback())
        {
            return;
        }
        let Ok(remote_port) = u16::try_from(remote_port) else {
            return;
        };
        let mut routes = self.0.lock().unwrap();
        let Some(route) = routes.get_mut(&remote_port) else {
            return;
        };
        while route.clients.try_join_next().is_some() {}
        if route.clients.len() >= 16 {
            return;
        }
        let local_port = route.local_port;
        route.clients.spawn(async move {
            let Ok(Ok(mut local)) =
                timeout(REQUEST_TIMEOUT, TcpStream::connect((LOOPBACK, local_port))).await
            else {
                return;
            };
            reply.accept().await;
            let mut stream = channel.into_stream();
            let _ = copy_bidirectional(&mut local, &mut stream).await;
        });
    }
}

impl Connection {
    pub(crate) async fn listen_remote(&self, remote_port: u16, local_port: u16) -> Result<u16> {
        let wire_port = timeout(
            REQUEST_TIMEOUT,
            self.handle.tcpip_forward(LOOPBACK, u32::from(remote_port)),
        )
        .await
        .map_err(|_| {
            Error::new(
                ErrorCode::DeliveryUnknown,
                "未收到远端监听回执，可关闭会话清理。",
            )
        })?
        .map_err(|error| forward_error(error, "未开启监听"))?;
        let port = if remote_port == 0 {
            u16::try_from(wire_port)
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    Error::new(ErrorCode::DeliveryUnknown, "服务器未返回有效的远端端口。")
                })?
        } else {
            remote_port
        };
        match self.remote_routes.0.lock().unwrap().entry(port) {
            Entry::Vacant(slot) => slot.insert(RemoteRoute {
                local_port,
                clients: JoinSet::new(),
            }),
            Entry::Occupied(_) => {
                return Err(Error::new(
                    ErrorCode::DeliveryUnknown,
                    "服务器返回了正在使用的监听端口，可关闭会话清理。",
                ));
            }
        };
        Ok(port)
    }

    pub(crate) async fn stop_remote(&self, remote_port: u16) -> Result<()> {
        self.remote_routes.0.lock().unwrap().remove(&remote_port);
        timeout(
            REQUEST_TIMEOUT,
            self.handle
                .cancel_tcpip_forward(LOOPBACK, u32::from(remote_port)),
        )
        .await
        .map_err(|_| {
            Error::new(
                ErrorCode::DeliveryUnknown,
                "未收到远端停止回执，可关闭会话清理。",
            )
        })?
        .map_err(|error| forward_error(error, "未停止监听"))?;
        Ok(())
    }
}

fn forward_error(error: russh::Error, rejection: &str) -> Error {
    if matches!(error, russh::Error::RequestDenied) {
        Error::new(ErrorCode::ChannelRejected, format!("服务器{rejection}。"))
    } else {
        Error::new(
            ErrorCode::DeliveryUnknown,
            format!("无法确认远端监听状态：{error}。可关闭会话清理。"),
        )
    }
}
