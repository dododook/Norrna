use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const MAGIC_TCP: &[u8; 8] = b"NORRNAMX";
pub const MAGIC_UDP: &[u8; 9] = b"NORRNAUDP";

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

pub fn encode_mux_header(owner: &str, target: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC_TCP);
    let ob = owner.as_bytes();
    let tb = target.as_bytes();
    out.extend_from_slice(&(ob.len() as u16).to_be_bytes());
    out.extend_from_slice(ob);
    out.extend_from_slice(&(tb.len() as u16).to_be_bytes());
    out.extend_from_slice(tb);
    out
}

pub fn decode_mux_header(buf: &[u8]) -> Option<(String, String, usize)> {
    if buf.len() < 12 || &buf[..8] != MAGIC_TCP {
        return None;
    }
    let ol = u16::from_be_bytes([buf[8], buf[9]]) as usize;
    if buf.len() < 10 + ol + 2 {
        return None;
    }
    let owner = String::from_utf8(buf[10..10 + ol].to_vec()).ok()?;
    let tl = u16::from_be_bytes([buf[10 + ol], buf[11 + ol]]) as usize;
    let start = 12 + ol;
    if buf.len() < start + tl {
        return None;
    }
    let target = String::from_utf8(buf[start..start + tl].to_vec()).ok()?;
    Some((owner, target, start + tl))
}

pub fn encode_udp_mux(owner: &str, target: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC_UDP);
    let ob = owner.as_bytes();
    let tb = target.as_bytes();
    out.extend_from_slice(&(ob.len() as u16).to_be_bytes());
    out.extend_from_slice(ob);
    out.extend_from_slice(&(tb.len() as u16).to_be_bytes());
    out.extend_from_slice(tb);
    out.extend_from_slice(payload);
    out
}

pub fn decode_udp_mux(buf: &[u8]) -> Option<(String, String, &[u8])> {
    if buf.len() < 13 || &buf[..9] != MAGIC_UDP {
        return None;
    }
    let ol = u16::from_be_bytes([buf[9], buf[10]]) as usize;
    if buf.len() < 11 + ol + 2 {
        return None;
    }
    let owner = String::from_utf8(buf[11..11 + ol].to_vec()).ok()?;
    let tl = u16::from_be_bytes([buf[11 + ol], buf[12 + ol]]) as usize;
    let start = 13 + ol;
    if buf.len() < start + tl {
        return None;
    }
    let target = String::from_utf8(buf[start..start + tl].to_vec()).ok()?;
    Some((owner, target, &buf[start + tl..]))
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
