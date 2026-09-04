use crate::relay::{spawn_instance, Running};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use norrna_proto::{read_frame, write_frame, Instance, InstanceConfig, WireMsg};

pub struct Agent {
    store: PathBuf,
    instances: Mutex<HashMap<String, Instance>>,
    running: Mutex<HashMap<String, Running>>,
}

impl Agent {
    pub async fn open(dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join("norrna.json");
        let instances = match tokio::fs::read(&path).await {
            Ok(b) => serde_json::from_slice::<Vec<Instance>>(&b).unwrap_or_default(),
            Err(_) => vec![],
        };
        let map: HashMap<_, _> = instances.into_iter().map(|i| (i.id.clone(), i)).collect();
        let agent = Self {
            store: path,
            instances: Mutex::new(map),
            running: Mutex::new(HashMap::new()),
        };
        let snapshot: Vec<_> = agent.instances.lock().await.values().cloned().collect();
        for inst in snapshot {
            if inst.auto_start && inst.status == "Running" {
                if let Err(e) = agent.start(&inst.id).await {
                    tracing::warn!("restore {} failed: {e}", inst.id);
                }
            }
        }
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
        let running = spawn_instance(inst.clone()).await?;
        self.running.lock().await.insert(id.to_string(), running);
        self.instances.lock().await.insert(id.to_string(), inst.clone());
        self.persist().await?;
        Ok(inst)
    }

    pub async fn stop(&self, id: &str) -> Result<Instance> {
        if let Some(r) = self.running.lock().await.remove(id) {
            r.abort();
        } else {
            anyhow::bail!("Instance is not running");
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

pub async fn run_agent(server: &str, key: &str, name: Option<String>, data_dir: PathBuf) -> Result<()> {
    let agent = Agent::open(&data_dir.join("instances")).await?;
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

async fn connect_once(server: &str, key: &str, name: &str, hostname: &str, agent: &Agent) -> Result<()> {
    tracing::info!("[agent] Connecting to {server}");
    let stream = tokio::time::timeout(Duration::from_secs(30), TcpStream::connect(server))
        .await
        .map_err(|_| anyhow::anyhow!("[agent] Connection timeout (30s)"))??;
    let _ = stream.set_nodelay(true);
    tracing::info!("[agent] TCP connected");
    let (r, w) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(r);
    let mut writer = tokio::io::BufWriter::new(w);

    tracing::info!("[agent] Sending auth...");
    write_frame(
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
    let resp = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut reader))
        .await
        .map_err(|_| anyhow::anyhow!("Auth response timeout"))??;
    match resp {
        WireMsg::AuthSuccess { .. } => tracing::info!("[agent] Authentication successful"),
        WireMsg::AuthFail { message } => anyhow::bail!("[agent] Authentication failed: {message}"),
        other => anyhow::bail!("[agent] Unexpected auth response type: {other:?}"),
    }
    tracing::info!("[agent] Entering command loop");

    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let (memory_usage, memory_total) = sample_memory();
                let _ = write_frame(&mut writer, &WireMsg::Ping).await;
                if write_frame(&mut writer, &WireMsg::Status {
                    cpu_usage: 0.0,
                    memory_usage,
                    memory_total,
                    ip: String::new(),
                    hostname: hostname.into(),
                }).await.is_err() {
                    anyhow::bail!("[agent] Heartbeat send failed");
                }
            }
            msg = read_frame(&mut reader) => {
                let msg = msg?;
                match msg {
                    WireMsg::Ping => {
                        write_frame(&mut writer, &WireMsg::Pong).await?;
                    }
                    WireMsg::Pong => {}
                    WireMsg::Command { req_id, command, instance_id, config, note } => {
                        tracing::info!("[agent] Received command: {command}");
                        let (success, message, data) = handle_cmd(agent, &command, instance_id, config, note).await;
                        write_frame(&mut writer, &WireMsg::Response { req_id, success, message, data }).await?;
                    }
                    _ => tracing::warn!("[agent] Unknown message type"),
                }
            }
        }
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
