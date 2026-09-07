use serde::{Deserialize, Serialize};
use norrna_proto::InstanceConfig;

pub const JWT_SECRET: &str = "your-jwt-secret-change-this-in-production-keep-it-very-secret";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_md5: String,
    pub role: String,
    pub created_at: String,
    pub last_login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsersData {
    pub users: Vec<User>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub role: String,
    pub exp: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub user_id: String,
    #[serde(default = "offline")]
    pub status: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub cpu_usage: f32,
    #[serde(default)]
    pub memory_usage: u64,
    #[serde(default)]
    pub memory_total: u64,
    #[serde(default)]
    pub last_seen: String,
    #[serde(default)]
    pub connected_at: String,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub multiplex_capable: bool,
    #[serde(default)]
    pub multiplex_port: u16,
    #[serde(default)]
    pub traffic_quota_bytes: u64,
    #[serde(default)]
    pub traffic_used_bytes: u64,
    #[serde(default)]
    pub traffic_month: String,
    #[serde(default)]
    pub last_rx_bytes: u64,
    #[serde(default)]
    pub last_tx_bytes: u64,
    #[serde(default)]
    pub quota_notified: bool,
    #[serde(default)]
    pub offline_notified: bool,
    #[serde(default)]
    pub realm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub telegram_bot_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
    #[serde(default)]
    pub telegram_enabled: bool,
    #[serde(default = "on")]
    pub notify_offline: bool,
    #[serde(default = "on")]
    pub notify_quota: bool,
}

fn on() -> bool {
    true
}

fn offline() -> String {
    "offline".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsData {
    pub agents: Vec<AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "disconnected")]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub user_id: String,
}

fn disconnected() -> String {
    "disconnected".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServersData {
    pub servers: Vec<ServerConfig>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub traffic_quota_gb: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentInstanceRequest {
    pub config: InstanceConfig,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_setup: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated: Option<bool>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            message: String::new(),
            data: Some(data),
            needs_setup: None,
            authenticated: None,
        }
    }
    pub fn msg(success: bool, message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success,
            message: message.into(),
            data: None,
            needs_setup: None,
            authenticated: None,
        }
    }
}


