use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

mod crypto;

pub use crypto::{split_crypto, EncReader, EncWriter};

pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const MAGIC_TCP: &[u8; 8] = b"NORRNAMX";
pub const MAGIC_UDP: &[u8; 9] = b"NORRNAUDP";
pub const MAGIC_UDP_ORIG: &[u8; 9] = b"ZELAY_UDP";
pub const MYSQL_VERSION: &[u8] = b"5.7.44-realm";
pub const MYSQL_PLUGIN: &[u8] = b"mysql_native_password";

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("Message too short")]
    TooShort,
    #[error("Incomplete packet")]
    Incomplete,
    #[error("Message too large")]
    TooLarge,
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Decryption failed")]
    Decrypt,
    #[error("Encryption failed")]
    Encrypt,
    #[error("Replay attack detected")]
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub listen: String,
    pub remote: String,
    #[serde(default)]
    pub extra_remotes: Vec<String>,
    #[serde(default)]
    pub multiplex_mode: i32,
    #[serde(default)]
    pub owner_user_id: Option<String>,
    #[serde(default)]
    pub final_target: Option<String>,
    #[serde(default)]
    pub network: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub config: InstanceConfig,
    pub status: String,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WireMsg {
    #[serde(rename = "auth")]
    Auth {
        api_key: String,
        #[serde(default)]
        hostname: String,
        #[serde(default)]
        name: String,
    },
    #[serde(rename = "auth_success")]
    AuthSuccess {
        #[serde(default)]
        agent_id: String,
        #[serde(default)]
        message: String,
    },
    #[serde(rename = "auth_fail")]
    AuthFail { message: String },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "status")]
    Status {
        #[serde(default)]
        cpu_usage: f32,
        #[serde(default)]
        memory_usage: u64,
        #[serde(default)]
        memory_total: u64,
        #[serde(default)]
        ip: String,
        #[serde(default)]
        hostname: String,
        #[serde(default)]
        multiplex_capable: bool,
        #[serde(default)]
        multiplex_port: u16,
        #[serde(default)]
        rx_bytes: u64,
        #[serde(default)]
        tx_bytes: u64,
        #[serde(default)]
        realm_version: String,
    },
    #[serde(rename = "command")]
    Command {
        req_id: String,
        command: String,
        #[serde(default)]
        instance_id: Option<String>,
        #[serde(default)]
        config: Option<InstanceConfig>,
        #[serde(default)]
        note: Option<String>,
    },
    #[serde(rename = "response")]
    Response {
        req_id: String,
        success: bool,
        #[serde(default)]
        message: String,
        #[serde(default)]
        data: serde_json::Value,
    },
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    msg: &WireMsg,
) -> Result<(), ProtoError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(ProtoError::TooLarge);
    }
    w.write_u32(body.len() as u32).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<WireMsg, ProtoError> {
    let len = r.read_u32().await?;
    if len == 0 {
        return Err(ProtoError::TooShort);
    }
    if len as usize > MAX_FRAME {
        return Err(ProtoError::TooLarge);
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

fn append_route(out: &mut Vec<u8>, owner: &str, target: &str) {
    let ob = owner.as_bytes();
    let tb = target.as_bytes();
    out.extend_from_slice(&(ob.len() as u16).to_be_bytes());
    out.extend_from_slice(ob);
    out.extend_from_slice(&(tb.len() as u16).to_be_bytes());
    out.extend_from_slice(tb);
}

fn take_route(buf: &[u8]) -> Option<(String, String, usize)> {
    if buf.len() < 4 {
        return None;
    }
    let ol = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if buf.len() < 2 + ol + 2 {
        return None;
    }
    let owner = String::from_utf8(buf[2..2 + ol].to_vec()).ok()?;
    let tl = u16::from_be_bytes([buf[2 + ol], buf[3 + ol]]) as usize;
    let start = 4 + ol;
    if buf.len() < start + tl {
        return None;
    }
    let target = String::from_utf8(buf[start..start + tl].to_vec()).ok()?;
    Some((owner, target, start + tl))
}

/// Original Zelay TCP mux: MySQL handshake v10 disguise (`5.7.44-realm` / `mysql_native_password`).
pub fn encode_mux_header(owner: &str, target: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(0x0a);
    payload.extend_from_slice(MYSQL_VERSION);
    payload.push(0);
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&[0u8; 8]);
    payload.push(0);
    payload.extend_from_slice(&0x8001u16.to_le_bytes());
    payload.push(33);
    payload.extend_from_slice(&2u16.to_le_bytes());
    payload.extend_from_slice(&0x0008u16.to_le_bytes());
    payload.push(21);
    payload.extend_from_slice(&[0u8; 10]);
    payload.extend_from_slice(&[0u8; 13]);
    payload.extend_from_slice(MYSQL_PLUGIN);
    payload.push(0);
    append_route(&mut payload, owner, target);
    let n = payload.len() as u32;
    let mut pkt = Vec::with_capacity(4 + payload.len());
    pkt.push((n & 0xff) as u8);
    pkt.push(((n >> 8) & 0xff) as u8);
    pkt.push(((n >> 16) & 0xff) as u8);
    pkt.push(0);
    pkt.extend_from_slice(&payload);
    pkt
}

pub fn decode_mux_header(buf: &[u8]) -> Option<(String, String, usize)> {
    if buf.len() >= 12 && &buf[..8] == MAGIC_TCP {
        return take_route(&buf[8..]).map(|(o, t, n)| (o, t, 8 + n));
    }
    if buf.len() < 8 || buf[4] != 0x0a {
        return None;
    }
    let inner = (buf[0] as usize) | ((buf[1] as usize) << 8) | ((buf[2] as usize) << 16);
    let total = 4 + inner;
    if buf.len() < total {
        return None;
    }
    let body = &buf[4..total];
    if !body.windows(MYSQL_VERSION.len()).any(|w| w == MYSQL_VERSION) {
        return None;
    }
    let needle = MYSQL_PLUGIN;
    let mut pos = None;
    for i in 0..body.len().saturating_sub(needle.len()) {
        if &body[i..i + needle.len()] == needle {
            pos = Some(i + needle.len());
            break;
        }
    }
    let pos = pos?;
    let mut rest = pos;
    if rest < body.len() && body[rest] == 0 {
        rest += 1;
    }
    let (owner, target, n) = take_route(&body[rest..])?;
    Some((owner, target, 4 + rest + n))
}

pub fn encode_udp_mux(owner: &str, target: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC_UDP_ORIG);
    append_route(&mut out, owner, target);
    out.extend_from_slice(payload);
    out
}

pub fn decode_udp_mux(buf: &[u8]) -> Option<(String, String, &[u8])> {
    if buf.len() < 13 {
        return None;
    }
    let magic_ok = &buf[..9] == MAGIC_UDP_ORIG || &buf[..9] == MAGIC_UDP;
    if !magic_ok {
        return None;
    }
    let (owner, target, n) = take_route(&buf[9..])?;
    Some((owner, target, &buf[9 + n..]))
}

pub fn parse_socket_addr(s: &str) -> anyhow::Result<std::net::SocketAddr> {
    if let Ok(addr) = s.parse() {
        return Ok(addr);
    }
    if let Some(port) = s.strip_prefix(":::") {
        return format!("[::]:{port}").parse::<std::net::SocketAddr>().map_err(Into::into);
    }
    if let Some(rest) = s.strip_prefix("*:") {
        return format!("0.0.0.0:{rest}").parse::<std::net::SocketAddr>().map_err(Into::into);
    }
    anyhow::bail!("invalid address: {s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_mux_roundtrip() {
        let pkt = encode_mux_header("user-1", "10.0.0.8:443");
        assert!(pkt.windows(MYSQL_VERSION.len()).any(|w| w == MYSQL_VERSION));
        assert!(pkt.windows(MYSQL_PLUGIN.len()).any(|w| w == MYSQL_PLUGIN));
        let (o, t, n) = decode_mux_header(&pkt).expect("decode");
        assert_eq!(o, "user-1");
        assert_eq!(t, "10.0.0.8:443");
        assert_eq!(n, pkt.len());
    }

    #[test]
    fn legacy_tcp_mux_still_decodes() {
        let mut pkt = MAGIC_TCP.to_vec();
        append_route(&mut pkt, "abc", "1.2.3.4:80");
        let (o, t, _) = decode_mux_header(&pkt).unwrap();
        assert_eq!(o, "abc");
        assert_eq!(t, "1.2.3.4:80");
    }

    #[test]
    fn udp_mux_roundtrip() {
        let pkt = encode_udp_mux("u", "9.9.9.9:53", b"hi");
        assert_eq!(&pkt[..9], MAGIC_UDP_ORIG);
        let (o, t, p) = decode_udp_mux(&pkt).unwrap();
        assert_eq!(o, "u");
        assert_eq!(t, "9.9.9.9:53");
        assert_eq!(p, b"hi");
    }
}
