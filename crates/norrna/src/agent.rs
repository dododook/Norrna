use crate::realm::{load_global, RealmEngine};
use crate::relay::{spawn_instance, Running};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use norrna_proto::{split_crypto, Instance, InstanceConfig, WireMsg};

enum RunKind {
    Overlay(Running),
    Realm,
}

pub struct Agent {
    store: PathBuf,
    conf_path: PathBuf,
    instances: Mutex<HashMap<String, Instance>>,
    running: Mutex<HashMap<String, RunKind>>,
    realm: Arc<RealmEngine>,
}

impl Agent {
    pub async fn open(dir: &Path, data_dir: &Path, conf: Option<PathBuf>) -> Result<Self> {
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join("norrna.json");
        let instances = match tokio::fs::read(&path).await {
            Ok(b) => serde_json::from_slice::<Vec<Instance>>(&b).unwrap_or_default(),
            Err(_) => vec![],
        };
        let map: HashMap<_, _> = instances.into_iter().map(|i| (i.id.clone(), i)).collect();
        let conf_path = conf.unwrap_or_else(|| data_dir.join("norrna.conf"));
        let realm = Arc::new(RealmEngine::discover(data_dir));
        let agent = Self {
            store: path,
            conf_path,
            instances: Mutex::new(map),
            running: Mutex::new(HashMap::new()),
            realm: realm.clone(),
        };
        let snapshot: Vec<_> = agent.instances.lock().await.values().cloned().collect();
        for inst in snapshot {
            if inst.auto_start && inst.status == "Running" {
                {
                    let mut g = agent.instances.lock().await;
                    if let Some(i) = g.get_mut(&inst.id) {
                        i.status = "Stopped".into();
                    }
                }
                if let Err(e) = agent.start(&inst.id).await {
                    tracing::warn!("restore {} failed: {e}", inst.id);
                    if let Some(i) = agent.instances.lock().await.get_mut(&inst.id) {
                        i.status = "Stopped".into();
                    }
                    let _ = agent.persist().await;
                }
            }
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                realm.respawn_if_dead().await;
            }
        });
        Ok(agent)
    }

    async fn persist(&self) -> Result<()> {
        let list: Vec<_> = self.instances.lock().await.values().cloned().collect();
        tokio::fs::write(&self.store, serde_json::to_vec_pretty(&list)?).await?;
        Ok(())
    }

    pub async fn list(&self) -> Vec<Instance> {
        self.instances.lock().await.values().cloned().collect()
    }

    pub async fn mux_info(&self) -> (bool, u16) {
        let running = self.running.lock().await;
        let insts = self.instances.lock().await;
        for (id, k) in running.iter() {
            if matches!(k, RunKind::Overlay(_)) {
                if let Some(i) = insts.get(id) {
                    if i.config.multiplex_mode == 1 {
                        let port = norrna_proto::parse_socket_addr(&i.config.listen)
                            .map(|a| a.port())
                            .unwrap_or(0);
                        return (true, port);
                    }
                }
            }
        }
        (true, 0)
    }

    pub async fn create(&self, mut config: InstanceConfig, note: Option<String>) -> Result<Instance> {
        if config.multiplex_mode == 1 && config.owner_user_id.is_none() {
            anyhow::bail!("Server mode requires owner_user_id");
        }
        if config.multiplex_mode == 2 {
            if config.owner_user_id.is_none() {
                anyhow::bail!("Client mode requires owner_user_id");
            }
            if config.final_target.is_none() {
                anyhow::bail!("Client mode requires final_target");
            }
        }
        let now = Utc::now().to_rfc3339();
        let inst = Instance {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            status: "Stopped".into(),
            note: note.unwrap_or_default(),
            auto_start: true,
            created_at: now.clone(),
            updated_at: now,
        };
        tracing::info!(
            "Creating instance ({})",
            match inst.config.multiplex_mode {
                1 => "Server mode",
                2 => "Client mode",
                _ => "Normal mode",
            }
        );
        self.instances.lock().await.insert(inst.id.clone(), inst.clone());
        self.persist().await?;
        self.start(&inst.id).await?;
        Ok(self
            .instances
            .lock()
            .await
            .get(&inst.id)
            .cloned()
            .unwrap())
    }

    pub async fn start(&self, id: &str) -> Result<Instance> {
        let inst = self
            .instances
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
        if self.running.lock().await.contains_key(id) {
            anyhow::bail!("Instance is already running");
        }
        let mut inst = inst;
        inst.status = "Running".into();
        inst.updated_at = Utc::now().to_rfc3339();
        if inst.config.multiplex_mode == 0 {
            let global = load_global(&self.conf_path);
            self.realm.start_one(id, &inst.config, &global).await?;
            self.running.lock().await.insert(id.to_string(), RunKind::Realm);
        } else {
            let running = spawn_instance(inst.clone()).await?;
            self.running
                .lock()
                .await
                .insert(id.to_string(), RunKind::Overlay(running));
        }
        self.instances.lock().await.insert(id.to_string(), inst.clone());
        self.persist().await?;
        Ok(inst)
    }

    pub async fn stop(&self, id: &str) -> Result<Instance> {
        let kind = self
            .running
            .lock()
            .await
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("Instance is not running"))?;
        match kind {
            RunKind::Overlay(r) => r.abort(),
            RunKind::Realm => self.realm.stop_one(id).await,
        }
        let mut g = self.instances.lock().await;
        let inst = g.get_mut(id).ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
        inst.status = "Stopped".into();
        inst.updated_at = Utc::now().to_rfc3339();
        let out = inst.clone();
        drop(g);
        self.persist().await?;
        Ok(out)
    }

    pub async fn restart(&self, id: &str) -> Result<Instance> {
        if self.running.lock().await.contains_key(id) {
            let _ = self.stop(id).await;
        }
        self.start(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        if self.running.lock().await.contains_key(id) {
            let _ = self.stop(id).await;
        }
        self.instances.lock().await.remove(id);
        self.persist().await?;
        Ok(())
    }

    pub async fn update(&self, id: &str, config: InstanceConfig, note: Option<String>) -> Result<Instance> {
        {
            let mut g = self.instances.lock().await;
            let inst = g.get_mut(id).ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
            inst.config = config;
            if let Some(n) = note {
                inst.note = n;
            }
            inst.updated_at = Utc::now().to_rfc3339();
        }
        self.persist().await?;
        if self.running.lock().await.contains_key(id) {
            let _ = self.stop(id).await;
            self.start(id).await
        } else {
            Ok(self.instances.lock().await.get(id).cloned().unwrap())
        }
    }

    pub async fn update_note(&self, id: &str, note: String) -> Result<Instance> {
        let mut g = self.instances.lock().await;
        let inst = g.get_mut(id).ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
        inst.note = note;
        inst.updated_at = Utc::now().to_rfc3339();
        let out = inst.clone();
        drop(g);
        self.persist().await?;
        Ok(out)
    }
}

pub async fn run_agent(
    server: &str,
    key: &str,
    name: Option<String>,
    data_dir: PathBuf,
    conf: Option<PathBuf>,
) -> Result<()> {
    let agent = Agent::open(&data_dir.join("instances"), &data_dir, conf).await?;
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".into());
    let name = name.unwrap_or_else(|| hostname.clone());

    loop {
        match connect_once(server, key, &name, &hostname, &agent).await {
            Ok(()) => tracing::warn!("[agent] Connection dead"),
            Err(e) => tracing::warn!("[agent] Connection failed: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

pub async fn run_passive(
    port: u16,
    key: &str,
    name: Option<String>,
    data_dir: PathBuf,
    conf: Option<PathBuf>,
) -> Result<()> {
    let agent = Arc::new(Agent::open(&data_dir.join("instances"), &data_dir, conf).await?);
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".into());
    let name = name.unwrap_or_else(|| hostname.clone());
    let bind: std::net::SocketAddr = format!("[::]:{port}").parse()?;
    let listener = TcpListener::bind(bind).await?;
    tracing::info!("Server started successfully");
    tracing::info!("[agent] passive API listening on {bind}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let agent = agent.clone();
        let key = key.to_string();
        let name = name.clone();
        let hostname = hostname.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_one(stream, &key, &name, &hostname, &agent).await {
                tracing::warn!("[agent] {peer} {e}");
            }
        });
    }
}

async fn connect_once(server: &str, key: &str, name: &str, hostname: &str, agent: &Agent) -> Result<()> {
    tracing::info!("[agent] Connecting to {server}");
    let stream = tokio::time::timeout(Duration::from_secs(30), TcpStream::connect(server))
        .await
        .map_err(|_| anyhow::anyhow!("[agent] Connection timeout (30s)"))??;
    let _ = stream.set_nodelay(true);
    tracing::info!("[agent] TCP connected");
    let (r, w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut writer = BufWriter::new(w);
    let (mut enc_r, mut enc_w) = split_crypto();

    tracing::info!("[agent] Sending auth...");
    enc_w
        .write_frame(
            &mut writer,
            &WireMsg::Auth {
                api_key: key.into(),
                hostname: hostname.into(),
                name: name.into(),
            },
        )
        .await?;
    tracing::info!("[agent] Auth sent");

    tracing::info!("[agent] Waiting for auth response...");
    let resp = tokio::time::timeout(Duration::from_secs(30), enc_r.read_frame(&mut reader))
        .await
        .map_err(|_| anyhow::anyhow!("Auth response timeout"))??;
    match resp {
        WireMsg::AuthSuccess { .. } => tracing::info!("[agent] Authentication successful"),
        WireMsg::AuthFail { message } => anyhow::bail!("[agent] Authentication failed: {message}"),
        other => anyhow::bail!("[agent] Unexpected auth response type: {other:?}"),
    }
    tracing::info!("[agent] Entering command loop");
    command_loop(&mut reader, &mut writer, enc_r, enc_w, agent, hostname).await
}

async fn serve_one(stream: TcpStream, key: &str, name: &str, hostname: &str, agent: &Agent) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let (r, w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut writer = BufWriter::new(w);
    let (mut enc_r, mut enc_w) = split_crypto();
    let first = tokio::time::timeout(Duration::from_secs(30), enc_r.read_frame(&mut reader))
        .await
        .map_err(|_| anyhow::anyhow!("Auth receive timeout"))??;
    match first {
        WireMsg::Auth { api_key, .. } if api_key == key => {}
        WireMsg::Auth { .. } => {
            let _ = enc_w
                .write_frame(
                    &mut writer,
                    &WireMsg::AuthFail {
                        message: "Invalid API key".into(),
                    },
                )
                .await;
            anyhow::bail!("Invalid API key");
        }
        _ => anyhow::bail!("Invalid API key"),
    }
    enc_w
        .write_frame(
            &mut writer,
            &WireMsg::AuthSuccess {
                agent_id: name.into(),
                message: "Authentication successful".into(),
            },
        )
        .await?;
    command_loop(&mut reader, &mut writer, enc_r, enc_w, agent, hostname).await
}

async fn command_loop<R, W>(
    reader: &mut R,
    writer: &mut W,
    mut enc_r: norrna_proto::EncReader,
    mut enc_w: norrna_proto::EncWriter,
    agent: &Agent,
    hostname: &str,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut cpu = CpuSampler::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let (memory_usage, memory_total) = sample_memory();
                let (multiplex_capable, multiplex_port) = agent.mux_info().await;
                let _ = enc_w.write_frame(writer, &WireMsg::Ping).await;
                if enc_w.write_frame(writer, &WireMsg::Status {
                    cpu_usage: cpu.sample(),
                    memory_usage,
                    memory_total,
                    ip: String::new(),
                    hostname: hostname.into(),
                    multiplex_capable,
                    multiplex_port,
                }).await.is_err() {
                    anyhow::bail!("[agent] Heartbeat send failed");
                }
            }
            msg = enc_r.read_frame(reader) => {
                let msg = msg?;
                match msg {
                    WireMsg::Ping => {
                        enc_w.write_frame(writer, &WireMsg::Pong).await?;
                    }
                    WireMsg::Pong => {}
                    WireMsg::Command { req_id, command, instance_id, config, note } => {
                        tracing::info!("[agent] Received command: {command}");
                        let (success, message, data) = handle_cmd(agent, &command, instance_id, config, note).await;
                        enc_w.write_frame(writer, &WireMsg::Response { req_id, success, message, data }).await?;
                    }
                    _ => tracing::warn!("[agent] Unknown message type"),
                }
            }
        }
    }
}

#[derive(Default)]
struct CpuSampler {
    prev_idle: u64,
    prev_total: u64,
}

impl CpuSampler {
    fn sample(&mut self) -> f32 {
        let Ok(s) = std::fs::read_to_string("/proc/stat") else {
            return 0.0;
        };
        let Some(line) = s.lines().next() else {
            return 0.0;
        };
        let mut nums = line.split_whitespace().skip(1).filter_map(|x| x.parse::<u64>().ok());
        let user = nums.next().unwrap_or(0);
        let nice = nums.next().unwrap_or(0);
        let system = nums.next().unwrap_or(0);
        let idle = nums.next().unwrap_or(0);
        let iowait = nums.next().unwrap_or(0);
        let irq = nums.next().unwrap_or(0);
        let softirq = nums.next().unwrap_or(0);
        let steal = nums.next().unwrap_or(0);
        let idle_all = idle + iowait;
        let total = user + nice + system + idle_all + irq + softirq + steal;
        let d_idle = idle_all.saturating_sub(self.prev_idle);
        let d_total = total.saturating_sub(self.prev_total);
        self.prev_idle = idle_all;
        self.prev_total = total;
        if d_total == 0 {
            return 0.0;
        }
        ((d_total - d_idle) as f32 / d_total as f32) * 100.0
    }
}

fn sample_memory() -> (u64, u64) {
    let Ok(s) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in s.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next()) {
            (Some("MemTotal:"), Some(v)) => total = v.parse::<u64>().unwrap_or(0) * 1024,
            (Some("MemAvailable:"), Some(v)) => avail = v.parse::<u64>().unwrap_or(0) * 1024,
            _ => {}
        }
    }
    (total.saturating_sub(avail), total)
}

fn ok_json(msg: &str, v: impl serde::Serialize) -> (bool, String, serde_json::Value) {
    (true, msg.into(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
}

fn err_json(msg: String) -> (bool, String, serde_json::Value) {
    (false, msg, serde_json::Value::Null)
}

async fn handle_cmd(
    agent: &Agent,
    command: &str,
    instance_id: Option<String>,
    config: Option<InstanceConfig>,
    note: Option<String>,
) -> (bool, String, serde_json::Value) {
    match command {
        "list_instances" => ok_json("ok", agent.list().await),
        "create_instance" => match config {
            Some(c) => match agent.create(c, note).await {
                Ok(i) => ok_json("Instance started successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            None => err_json("Missing config parameter".into()),
        },
        "start_instance" => match instance_id {
            Some(id) => match agent.start(&id).await {
                Ok(i) => ok_json("Instance started successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            None => err_json("Missing instance_id parameter".into()),
        },
        "stop_instance" => match instance_id {
            Some(id) => match agent.stop(&id).await {
                Ok(i) => ok_json("Instance stopped successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            None => err_json("Missing instance_id parameter".into()),
        },
        "restart_instance" => match instance_id {
            Some(id) => match agent.restart(&id).await {
                Ok(i) => ok_json("Instance restarted successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            None => err_json("Missing instance_id parameter".into()),
        },
        "delete_instance" => match instance_id {
            Some(id) => match agent.delete(&id).await {
                Ok(()) => ok_json("Instance deleted successfully", serde_json::Value::Null),
                Err(e) => err_json(e.to_string()),
            },
            None => err_json("Missing instance_id parameter".into()),
        },
        "update_instance" => match (instance_id, config) {
            (Some(id), Some(c)) => match agent.update(&id, c, note).await {
                Ok(i) => ok_json("Instance updated and restarted successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            (None, _) => err_json("Missing instance_id parameter".into()),
            (_, None) => err_json("Missing 'config' parameter".into()),
        },
        "update_note" => match (instance_id, note) {
            (Some(id), Some(n)) => match agent.update_note(&id, n).await {
                Ok(i) => ok_json("Note updated successfully", i),
                Err(e) => err_json(e.to_string()),
            },
            (_, None) => err_json("Missing note parameter".into()),
            (None, _) => err_json("Missing instance_id parameter".into()),
        },
        "get_instance" => match instance_id {
            Some(id) => match agent.list().await.into_iter().find(|i| i.id == id) {
                Some(i) => ok_json("ok", i),
                None => err_json("Instance not found".into()),
            },
            None => err_json("Missing instance_id parameter".into()),
        },
        _ => err_json("Unknown error".into()),
    }
}
