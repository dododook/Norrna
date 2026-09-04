use crate::auth::{
    clear_cookie, cookie_header, make_token, parse_token, password_md5, token_from_cookie,
};
use crate::hub::AgentHub;
use crate::models::*;
use crate::storage::Storage;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use norrna_proto::WireMsg;

const DASHBOARD: &str = include_str!("../static/dashboard.html");
const LOGIN: &str = include_str!("../static/login.html");
const SETUP: &str = include_str!("../static/setup.html");

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
    pub hub: AgentHub,
    pub agent_port: u16,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(page_root))
        .route("/login", get(|| async { Html(LOGIN) }))
        .route("/setup", get(|| async { Html(SETUP) }))
        .route("/norrna_agent.sh", get(serve_agent_script))
        .route("/norrna", get(serve_agent_binary))
        .route("/api/auth/check-setup", get(check_setup))
        .route("/api/auth/setup", post(setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/check", get(check_auth))
        .route("/api/auth/me", get(me))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/change-password", post(change_password))
        .route("/api/server-info", get(server_info))
        .route("/api/servers", get(list_servers).post(create_server))
        .route("/api/servers/:id", axum::routing::delete(delete_server))
        .route("/api/servers/:id/connect", post(connect_server))
        .route(
            "/api/servers/:id/instances",
            get(list_server_instances).post(create_server_instance),
        )
        .route(
            "/api/servers/:server_id/instances/:instance_id",
            axum::routing::put(update_server_instance).delete(delete_server_instance),
        )
        .route(
            "/api/servers/:server_id/instances/:instance_id/start",
            post(start_server_instance),
        )
        .route(
            "/api/servers/:server_id/instances/:instance_id/stop",
            post(stop_server_instance),
        )
        .route(
            "/api/servers/:server_id/instances/:instance_id/restart",
            post(restart_server_instance),
        )
        .route(
            "/api/servers/:server_id/instances/:instance_id/note",
            post(update_server_note),
        )
        .route("/api/agents/multiplex-capable", get(list_mux_agents))
        .route("/api/agents/:id/multiplex", post(set_agent_mux))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/:id",
            axum::routing::put(update_agent).delete(delete_agent),
        )
        .route("/api/agents/:id/full", get(agent_full))
        .route(
            "/api/agents/:id/instances",
            get(list_instances).post(create_instance),
        )
        .route(
            "/api/agents/:agent_id/instances/:instance_id",
            axum::routing::put(update_instance).delete(delete_instance),
        )
        .route(
            "/api/agents/:agent_id/instances/:instance_id/start",
            post(start_instance),
        )
        .route(
            "/api/agents/:agent_id/instances/:instance_id/stop",
            post(stop_instance),
        )
        .route(
            "/api/agents/:agent_id/instances/:instance_id/restart",
            post(restart_instance),
        )
        .route(
            "/api/agents/:agent_id/instances/:instance_id/note",
            post(update_note),
        )
        .layer(CorsLayer::permissive())
        .with_state(state)
}

const AGENT_SH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/norrna_agent.sh"
));

async fn serve_agent_script(headers: HeaderMap) -> Response {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:3000");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let url = format!("{proto}://{host}/norrna");
    let body = AGENT_SH.replace(
        r#"PANEL_BINARY_URL="""#,
        &format!(r#"PANEL_BINARY_URL="{url}""#),
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

async fn serve_agent_binary() -> Response {
    let mut cands = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("norrna"));
        }
    }
    cands.push(std::path::PathBuf::from("/etc/norrna-manager/norrna"));
    cands.push(std::path::PathBuf::from("/opt/norrna/norrna"));
    for p in cands {
        if let Ok(bytes) = tokio::fs::read(&p).await {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/octet-stream")],
                bytes,
            )
                .into_response();
        }
    }
    (
        StatusCode::NOT_FOUND,
        "norrna binary not found; copy it next to norrna-manager (e.g. /etc/norrna-manager/norrna)",
    )
        .into_response()
}

async fn page_root(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let users = st.storage.users().await;
    if users.is_empty() {
        return Redirect::to("/setup").into_response();
    }
    if current_user(&st, &headers).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    Html(DASHBOARD).into_response()
}

async fn current_user(st: &AppState, headers: &HeaderMap) -> Option<User> {
    let raw = headers.get(header::COOKIE)?.to_str().ok();
    let token = token_from_cookie(raw)?;
    let claims = parse_token(&token)?;
    st.storage
        .users()
        .await
        .into_iter()
        .find(|u| u.id == claims.sub)
}

async fn require_user(st: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    current_user(st, headers).await.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::msg(false, "Authentication required")),
        )
            .into_response()
    })
}

async fn check_setup(State(st): State<AppState>) -> impl IntoResponse {
    let needs = st.storage.users().await.is_empty();
    Json(serde_json::json!({"success": true, "needs_setup": needs}))
}

async fn setup(State(st): State<AppState>, Json(req): Json<SetupRequest>) -> Response {
    if !st.storage.users().await.is_empty() {
        return Json(ApiResponse::<()>::msg(false, "System already initialized")).into_response();
    }
    if req.username.len() < 3 {
        return Json(ApiResponse::<()>::msg(
            false,
            "Username must be at least 3 characters",
        ))
        .into_response();
    }
    if req.password.len() < 6 {
        return Json(ApiResponse::<()>::msg(
            false,
            "Password must be at least 6 characters",
        ))
        .into_response();
    }
    let now = chrono::Utc::now().to_rfc3339();
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username: req.username,
        password_md5: password_md5(&req.password),
        role: "Admin".into(),
        created_at: now.clone(),
        last_login: now,
    };
    let token = match make_token(&user) {
        Ok(t) => t,
        Err(e) => {
            return Json(ApiResponse::<()>::msg(false, e.to_string())).into_response();
        }
    };
    let _ = st.storage.save_users(vec![user]).await;
    cookie_json(
        true,
        "Admin account created successfully",
        &cookie_header(&token),
    )
}

async fn login(State(st): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    let mut users = st.storage.users().await;
    if users.is_empty() {
        return Json(ApiResponse::<()>::msg(
            false,
            "No users in system. Please visit /setup to create admin account",
        ))
        .into_response();
    }
    let hash = password_md5(&req.password);
    let Some(pos) = users
        .iter()
        .position(|u| u.username == req.username && u.password_md5 == hash)
    else {
        return Json(ApiResponse::<()>::msg(false, "Invalid username or password"))
            .into_response();
    };
    users[pos].last_login = chrono::Utc::now().to_rfc3339();
    let user = users[pos].clone();
    let _ = st.storage.save_users(users).await;
    let token = match make_token(&user) {
        Ok(t) => t,
        Err(e) => return Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    };
    cookie_json(true, "ok", &cookie_header(&token))
}

fn cookie_json(success: bool, message: &str, cookie: &str) -> Response {
    let mut res = Json(ApiResponse::<()>::msg(success, message)).into_response();
    if let Ok(v) = HeaderValue::from_str(cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

async fn check_auth(State(st): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let authenticated = current_user(&st, &headers).await.is_some();
    Json(serde_json::json!({"authenticated": authenticated}))
}

async fn me(State(st): State<AppState>, headers: HeaderMap) -> Response {
    match require_user(&st, &headers).await {
        Ok(u) => Json(serde_json::json!({
            "success": true,
            "user": {
                "id": u.id,
                "username": u.username,
                "role": u.role
            }
        }))
        .into_response(),
        Err(r) => r,
    }
}

async fn logout() -> Response {
    cookie_json(true, "Logged out successfully", &clear_cookie())
}

async fn change_password(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if req.new_password.len() < 6 {
        return Json(ApiResponse::<()>::msg(
            false,
            "Password must be at least 6 characters",
        ))
        .into_response();
    }
    if password_md5(&req.old_password) != user.password_md5 {
        return Json(ApiResponse::<()>::msg(false, "Invalid old password")).into_response();
    }
    let mut users = st.storage.users().await;
    if let Some(u) = users.iter_mut().find(|u| u.id == user.id) {
        u.password_md5 = password_md5(&req.new_password);
    }
    let _ = st.storage.save_users(users).await;
    Json(ApiResponse::<()>::msg(true, "Password changed successfully")).into_response()
}

async fn server_info(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    Json(ApiResponse::ok(serde_json::json!({
        "agent_port": st.agent_port
    })))
    .into_response()
}

fn public_agent(a: &AgentConfig) -> serde_json::Value {
    serde_json::json!({
        "id": a.id,
        "name": a.name,
        "status": a.status,
        "ip": a.ip,
        "hostname": a.hostname,
        "cpu_usage": a.cpu_usage,
        "memory_usage": a.memory_usage,
        "memory_total": a.memory_total,
        "last_seen": a.last_seen,
        "connected_at": a.connected_at,
        "created_at": a.created_at,
        "updated_at": a.updated_at,
        "multiplex_capable": a.multiplex_capable,
        "user_id": a.user_id,
    })
}

async fn list_agents(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut agents = st.storage.agents().await;
    for a in &mut agents {
        if !st.hub.is_online(&a.id).await {
            a.status = "offline".into();
        } else {
            a.status = "online".into();
        }
    }
    let visible: Vec<_> = agents
        .iter()
        .filter(|a| a.user_id == user.id || user.role == "Admin")
        .map(public_agent)
        .collect();
    Json(ApiResponse::ok(visible)).into_response()
}

async fn create_agent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAgentRequest>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut agents = st.storage.agents().await;
    if agents.iter().any(|a| a.name == req.name) {
        return Json(ApiResponse::<()>::msg(false, "Agent name already exists")).into_response();
    }
    let now = chrono::Utc::now().to_rfc3339();
    let agent = AgentConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name,
        api_key: req.api_key,
        user_id: user.id,
        status: "offline".into(),
        ip: String::new(),
        hostname: String::new(),
        cpu_usage: 0.0,
        memory_usage: 0,
        memory_total: 0,
        last_seen: String::new(),
        connected_at: String::new(),
        created_at: now.clone(),
        updated_at: now,
        multiplex_capable: true,
        multiplex_port: 0,
    };
    agents.push(agent.clone());
    let _ = st.storage.save_agents(agents).await;
    Json(ApiResponse::ok(public_agent(&agent))).into_response()
}

async fn agent_full(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let agents = st.storage.agents().await;
    match agents.into_iter().find(|a| a.id == id) {
        Some(a) if a.user_id == user.id || user.role == "Admin" => {
            Json(ApiResponse::ok(a)).into_response()
        }
        _ => Json(ApiResponse::<()>::msg(false, "Agent not found")).into_response(),
    }
}

async fn update_agent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st
        .storage
        .update_agent(&id, |a| {
            if let Some(n) = req.name.clone() {
                a.name = n;
            }
            if let Some(k) = req.api_key.clone() {
                if k != "********" && !k.is_empty() {
                    a.api_key = k;
                }
            }
            a.updated_at = chrono::Utc::now().to_rfc3339();
        })
        .await
    {
        Ok(Some(a)) => Json(ApiResponse::ok(public_agent(&a))).into_response(),
        _ => Json(ApiResponse::<()>::msg(false, "Agent not found")).into_response(),
    }
}

async fn delete_agent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let mut agents = st.storage.agents().await;
    let before = agents.len();
    agents.retain(|a| a.id != id);
    if agents.len() == before {
        return Json(ApiResponse::<()>::msg(false, "Agent not found")).into_response();
    }
    let _ = st.storage.save_agents(agents).await;
    Json(ApiResponse::<()>::msg(true, "ok")).into_response()
}

fn unwrap_response(msg: WireMsg) -> Response {
    match msg {
        WireMsg::Response {
            success,
            message,
            data,
            ..
        } => Json(serde_json::json!({
            "success": success,
            "message": message,
            "data": data
        }))
        .into_response(),
        _ => Json(ApiResponse::<()>::msg(false, "Invalid response from agent")).into_response(),
    }
}

async fn list_instances(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st.hub.command(&id, "list_instances", None, None, None).await {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn create_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateAgentInstanceRequest>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut cfg = req.config;
    if cfg.multiplex_mode != 0 && cfg.owner_user_id.is_none() {
        cfg.owner_user_id = Some(user.id);
    }
    match st
        .hub
        .command(&id, "create_instance", None, Some(cfg), req.note)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to create instance: {e}")))
            .into_response(),
    }
}

async fn update_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
    Json(req): Json<CreateAgentInstanceRequest>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st
        .hub
        .command(
            &agent_id,
            "update_instance",
            Some(instance_id),
            Some(req.config),
            req.note,
        )
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to update instance: {e}")))
            .into_response(),
    }
}

async fn delete_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st
        .hub
        .command(&agent_id, "delete_instance", Some(instance_id), None, None)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to delete instance: {e}")))
            .into_response(),
    }
}

async fn inst_cmd(st: &AppState, agent_id: &str, instance_id: String, cmd: &str) -> Response {
    match st
        .hub
        .command(agent_id, cmd, Some(instance_id), None, None)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn start_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &agent_id, instance_id, "start_instance").await
}

async fn stop_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &agent_id, instance_id, "stop_instance").await
}

async fn restart_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &agent_id, instance_id, "restart_instance").await
}

async fn update_note(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, instance_id)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match st
        .hub
        .command(&agent_id, "update_note", Some(instance_id), None, note)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn list_servers(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut servers = st.storage.servers().await;
    for s in &mut servers {
        s.status = if st.hub.is_online(&s.id).await {
            "connected".into()
        } else {
            "disconnected".into()
        };
    }
    let visible: Vec<_> = servers
        .into_iter()
        .filter(|s| s.user_id == user.id || user.role == "Admin")
        .collect();
    Json(ApiResponse::ok(visible)).into_response()
}

async fn create_server(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateServerRequest>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut servers = st.storage.servers().await;
    let now = chrono::Utc::now().to_rfc3339();
    let s = ServerConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name,
        host: req.host,
        port: req.port,
        api_key: req.api_key,
        status: "disconnected".into(),
        created_at: now.clone(),
        updated_at: now,
        user_id: user.id,
    };
    servers.push(s.clone());
    let _ = st.storage.save_servers(servers).await;
    Json(ApiResponse::ok(s)).into_response()
}

async fn delete_server(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let mut servers = st.storage.servers().await;
    servers.retain(|s| s.id != id);
    let _ = st.storage.save_servers(servers).await;
    Json(ApiResponse::<()>::msg(true, "ok")).into_response()
}

async fn connect_server(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let servers = st.storage.servers().await;
    let Some(s) = servers.iter().find(|s| s.id == id).cloned() else {
        return Json(ApiResponse::<()>::msg(false, "Server not found")).into_response();
    };
    match st
        .hub
        .connect_passive(&s.id, &s.host, s.port, &s.api_key)
        .await
    {
        Ok(()) => {
            let mut servers = st.storage.servers().await;
            if let Some(x) = servers.iter_mut().find(|x| x.id == id) {
                x.status = "connected".into();
                let _ = st.storage.save_servers(servers).await;
            }
            Json(ApiResponse::<()>::msg(true, "Connected successfully")).into_response()
        }
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn list_server_instances(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st.hub.command(&id, "list_instances", None, None, None).await {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn create_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateAgentInstanceRequest>,
) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut cfg = req.config;
    if cfg.multiplex_mode != 0 && cfg.owner_user_id.is_none() {
        cfg.owner_user_id = Some(user.id);
    }
    match st
        .hub
        .command(&id, "create_instance", None, Some(cfg), req.note)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to create instance: {e}")))
            .into_response(),
    }
}

async fn update_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
    Json(req): Json<CreateAgentInstanceRequest>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st
        .hub
        .command(
            &server_id,
            "update_instance",
            Some(instance_id),
            Some(req.config),
            req.note,
        )
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to update instance: {e}")))
            .into_response(),
    }
}

async fn delete_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    match st
        .hub
        .command(&server_id, "delete_instance", Some(instance_id), None, None)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, format!("Failed to delete instance: {e}")))
            .into_response(),
    }
}

async fn start_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &server_id, instance_id, "start_instance").await
}

async fn stop_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &server_id, instance_id, "stop_instance").await
}

async fn restart_server_instance(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    inst_cmd(&st, &server_id, instance_id, "restart_instance").await
}

async fn update_server_note(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((server_id, instance_id)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match st
        .hub
        .command(&server_id, "update_note", Some(instance_id), None, note)
        .await
    {
        Ok(m) => unwrap_response(m),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}

async fn list_mux_agents(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_user(&st, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let agents = st.storage.agents().await;
    let list: Vec<_> = agents
        .iter()
        .filter(|a| a.multiplex_capable && (a.user_id == user.id || user.role == "Admin"))
        .map(public_agent)
        .collect();
    Json(ApiResponse::ok(list)).into_response()
}

async fn set_agent_mux(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Err(r) = require_user(&st, &headers).await {
        return r;
    }
    let capable = body
        .get("multiplex_capable")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let port = body
        .get("multiplex_port")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    match st
        .storage
        .update_agent(&id, |a| {
            a.multiplex_capable = capable;
            a.multiplex_port = port;
        })
        .await
    {
        Ok(Some(_)) => Json(ApiResponse::<()>::msg(true, "Agent multiplex capability updated")).into_response(),
        Ok(None) => Json(ApiResponse::<()>::msg(false, "Agent not found")).into_response(),
        Err(e) => Json(ApiResponse::<()>::msg(false, e.to_string())).into_response(),
    }
}
