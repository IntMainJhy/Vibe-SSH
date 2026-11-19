//! Outbound CONNECT tunnels selected by the trusted host configuration.
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use vibe_ssh_protocol::ErrorCode;

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    HttpConnect,
    Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxySettings {
    pub kind: ProxyKind,
    pub address: String,
    pub port: u16,
}

impl ProxySettings {
    pub(crate) fn validate(&self) -> Result<()> {
        if !valid_host(&self.address) || self.port == 0 {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "请填写有效的代理地址和端口。",
            ));
        }
        Ok(())
    }

    pub(crate) async fn connect(&self, host: &str, port: u16) -> Result<BufReader<TcpStream>> {
        self.validate()?;
        if !valid_host(host) || port == 0 {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "代理目标需要有效的 IP 或 ASCII 主机名与端口。",
            ));
        }
        let socket = TcpStream::connect((unbracket(&self.address), self.port))
            .await
            .map_err(|error| failed(format!("无法连接代理：{error}")))?;
        let mut stream = BufReader::new(socket);
        match self.kind {
            ProxyKind::HttpConnect => http_connect(&mut stream, host, port).await?,
            ProxyKind::Socks5 => socks_connect(&mut stream, host, port).await?,
        }
        Ok(stream)
    }
}

fn unbracket(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
}

fn valid_host(host: &str) -> bool {
    unbracket(host).parse::<IpAddr>().is_ok()
        || (!host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
}

fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::ConnectionFailed, message)
}

fn proxy_io(error: std::io::Error) -> Error {
    failed(format!("代理握手未完成：{error}"))
}

async fn http_connect(stream: &mut BufReader<TcpStream>, host: &str, port: u16) -> Result<()> {
    let host = unbracket(host);
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(proxy_io)?;
    stream.flush().await.map_err(proxy_io)?;
    // Preserve unread SSH identification bytes buffered after the end of the HTTP header.
    let mut response = stream.take(16 * 1024);
    let mut line = Vec::new();
    loop {
        response
            .read_until(b'\n', &mut line)
            .await
            .map_err(proxy_io)?;
        let first = std::str::from_utf8(&line).map_err(|_| failed("HTTP 代理返回无效状态行。"))?;
        let mut fields = first
            .strip_suffix("\r\n")
            .unwrap_or_default()
            .split_ascii_whitespace();
        let version = fields.next().unwrap_or_default();
        let code = fields
            .next()
            .filter(|code| code.len() == 3)
            .and_then(|code| code.parse::<u16>().ok());
        if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || code.is_none() {
            return Err(failed("HTTP 代理返回无效状态行。"));
        }
        let code = code.expect("validated status");
        loop {
            line.clear();
            response
                .read_until(b'\n', &mut line)
                .await
                .map_err(proxy_io)?;
            if !line.ends_with(b"\r\n") {
                return Err(failed("HTTP 代理响应未结束或超出 16 KiB。"));
            }
            if line == b"\r\n" {
                break;
            }
        }
        if (100..200).contains(&code) {
            line.clear();
            continue;
        }
        return match code {
            200..=299 => Ok(()),
            407 => Err(failed("HTTP 代理要求认证。请选择允许此连接的代理。")),
            _ => Err(failed(format!("HTTP 代理拒绝连接（{code}）。"))),
        };
    }
}

async fn socks_connect(stream: &mut BufReader<TcpStream>, host: &str, port: u16) -> Result<()> {
    stream.write_all(&[5, 1, 0]).await.map_err(proxy_io)?;
    stream.flush().await.map_err(proxy_io)?;
    let mut method = [0; 2];
    stream.read_exact(&mut method).await.map_err(proxy_io)?;
    if method != [5, 0] {
        return Err(failed("SOCKS5 代理未接受免认证方式。"));
    }
    let host = unbracket(host);
    let mut request = vec![5, 1, 0];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend(address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend(address.octets());
        }
        Err(_) => {
            request.extend([3, host.len() as u8]);
            request.extend(host.as_bytes());
        }
    }
    request.extend(port.to_be_bytes());
    stream.write_all(&request).await.map_err(proxy_io)?;
    stream.flush().await.map_err(proxy_io)?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).await.map_err(proxy_io)?;
    if reply[0] != 5 || reply[2] != 0 {
        return Err(failed("SOCKS5 代理返回无效应答。"));
    }
    if reply[1] != 0 {
        return Err(failed(format!("SOCKS5 代理拒绝连接（{}）。", reply[1])));
    }
    let address_len = match reply[3] {
        1 => 4,
        3 => stream.read_u8().await.map_err(proxy_io)? as usize,
        4 => 16,
        _ => return Err(failed("SOCKS5 代理返回未知地址类型。")),
    };
    let mut bound = [0; 257];
    stream
        .read_exact(&mut bound[..address_len + 2])
        .await
        .map_err(proxy_io)?;
    Ok(())
}
