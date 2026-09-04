use crate::storage::Storage;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use norrna_proto::{split_crypto, EncReader, EncWriter, InstanceConfig, WireMsg};

#[derive(Clone)]
pub struct AgentHub {
    tx_map: Arc<Mutex<HashMap<String, mpsc::Sender<WireMsg>>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<WireMsg>>>>,
    storage: Arc<Storage>,
}

impl AgentHub {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            tx_map: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            storage,
        }
    }

    pub async fn is_online(&self, agent_id: &str) -> bool {
        self.tx_map.lock().await.contains_key(agent_id)
    }

    pub async fn serve(self, bind: std::net::SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(bind).await?;
        tracing::info!("Agent TCP listening on {bind}");
        loop {
            let (stream, peer) = listener.accept().await?;
            let hub = self.clone();
            tokio::spawn(async move {
                if let Err(e) = hub.handle_inbound(stream, peer).await {
                    tracing::warn!("Agent TCP {peer} error: {e}");
                }
            });
        }
    }

    async fn handle_inbound(&self, stream: TcpStream, peer: std::net::SocketAddr) -> Result<()> {
        let _ = stream.set_nodelay(true);
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);
        let (mut enc_r, mut enc_w) = split_crypto();

        let first = tokio::time::timeout(std::time::Duration::from_secs(30), enc_r.read_frame(&mut reader))
            .await
            .map_err(|_| anyhow::anyhow!("Auth receive timeout"))??;

        let (api_key, hostname) = match first {
            WireMsg::Auth {
                api_key, hostname, ..
            } => (api_key, hostname),
            _ => anyhow::bail!("Invalid API key"),
        };
        if api_key.is_empty() {
            let _ = enc_w
                .write_frame(
                    &mut writer,
                    &WireMsg::AuthFail {
                        message: "Missing api_key".into(),
                    },
                )
                .await;
            anyhow::bail!("Missing api_key");
        }

        let agents = self.storage.agents().await;
        let Some(agent) = agents.into_iter().find(|a| a.api_key == api_key) else {
            let _ = enc_w
                .write_frame(
                    &mut writer,
                    &WireMsg::AuthFail {
                        message: "Invalid API key".into(),
                    },
                )
                .await;
            anyhow::bail!("Invalid API key");
        };
        let agent_id = agent.id.clone();
        let now = chrono::Utc::now().to_rfc3339();
        self.storage
            .update_agent(&agent_id, |a| {
                a.status = "online".into();
                a.ip = peer.ip().to_string();
                a.hostname = hostname;
                a.connected_at = now.clone();
                a.last_seen = now.clone();
                a.updated_at = now;
            })
            .await?;

        enc_w
            .write_frame(
                &mut writer,
                &WireMsg::AuthSuccess {
                    agent_id: agent_id.clone(),
                    message: "Authentication successful".into(),
                },
            )
            .await?;

        self.pump(agent_id, reader, writer, enc_r, enc_w, true).await
    }

    pub async fn connect_passive(
        &self,
        server_id: &str,
        host: &str,
        port: u16,
        api_key: &str,
    ) -> Result<()> {
        let addr = format!("{host}:{port}");
        tracing::info!("connecting to passive agent {addr}");
        let stream = tokio::time::timeout(std::time::Duration::from_secs(15), TcpStream::connect(&addr))
            .await
            .map_err(|_| anyhow::anyhow!("Connection timeout"))??;
        let _ = stream.set_nodelay(true);
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);
        let (mut enc_r, mut enc_w) = split_crypto();
        enc_w
            .write_frame(
                &mut writer,
                &WireMsg::Auth {
                    api_key: api_key.into(),
                    hostname: String::new(),
                    name: "norrna-manager".into(),
                },
            )
            .await?;
        let resp = tokio::time::timeout(std::time::Duration::from_secs(15), enc_r.read_frame(&mut reader))
            .await
            .map_err(|_| anyhow::anyhow!("Auth response timeout"))??;
        match resp {
            WireMsg::AuthSuccess { .. } => {}
            WireMsg::AuthFail { message } => anyhow::bail!("Authentication failed: {message}"),
            other => anyhow::bail!("Unexpected auth response: {other:?}"),
        }
        let hub = self.clone();
        let sid = server_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = hub.pump(sid, reader, writer, enc_r, enc_w, false).await {
                tracing::warn!("passive agent connection ended: {e}");
            }
        });
        for _ in 0..50 {
            if self.is_online(server_id).await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        anyhow::bail!("connected but session did not come online")
    }

    async fn pump(
        &self,
        id: String,
        mut reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
        mut writer: BufWriter<tokio::net::tcp::OwnedWriteHalf>,
        mut enc_r: EncReader,
        mut enc_w: EncWriter,
        is_agent: bool,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<WireMsg>(128);
        self.tx_map.lock().await.insert(id.clone(), tx);

        let pending = self.pending.clone();
        let storage = self.storage.clone();
        let aid = id.clone();
        let read_task = tokio::spawn(async move {
            loop {
                match enc_r.read_frame(&mut reader).await {
                    Ok(WireMsg::Pong | WireMsg::Ping) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        if is_agent {
                            let _ = storage
                                .update_agent(&aid, |a| {
                                    a.last_seen = now;
                                    a.status = "online".into();
                                })
                                .await;
                        }
                    }
                    Ok(WireMsg::Status {
                        cpu_usage,
                        memory_usage,
                        memory_total,
                        ip,
                        hostname,
                        multiplex_capable,
                        multiplex_port,
                    }) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        if is_agent {
                            let _ = storage
                                .update_agent(&aid, |a| {
                                    a.cpu_usage = cpu_usage;
                                    a.memory_usage = memory_usage;
                                    a.memory_total = memory_total;
                                    a.multiplex_capable = multiplex_capable;
                                    if multiplex_port != 0 {
                                        a.multiplex_port = multiplex_port;
                                    }
                                    if !ip.is_empty() {
                                        a.ip = ip;
                                    }
                                    if !hostname.is_empty() {
                                        a.hostname = hostname;
                                    }
                                    a.last_seen = now;
                                    a.status = "online".into();
                                })
                                .await;
                        }
                    }
                    Ok(msg @ WireMsg::Response { .. }) => {
                        if let WireMsg::Response { req_id, .. } = &msg {
                            if let Some(s) = pending.lock().await.remove(req_id) {
                                let _ = s.send(msg);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });

        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if enc_w.write_frame(&mut writer, &msg).await.is_err() {
                    break;
                }
            }
        });

        let _ = read_task.await;
        self.tx_map.lock().await.remove(&id);
        if is_agent {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = self
                .storage
                .update_agent(&id, |a| {
                    a.status = "offline".into();
                    a.last_seen = now;
                })
                .await;
        } else {
            let mut servers = self.storage.servers().await;
            if let Some(s) = servers.iter_mut().find(|s| s.id == id) {
                s.status = "disconnected".into();
                let _ = self.storage.save_servers(servers).await;
            }
        }
        write_task.abort();
        Ok(())
    }

    pub async fn command(
        &self,
        agent_id: &str,
        command: &str,
        instance_id: Option<String>,
        config: Option<InstanceConfig>,
        note: Option<String>,
    ) -> Result<WireMsg> {
        let tx = self
            .tx_map
            .lock()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Agent is not connected"))?;
        let req_id = uuid::Uuid::new_v4().to_string();
        let (rtx, rrx) = oneshot::channel();
        self.pending.lock().await.insert(req_id.clone(), rtx);
        tx.send(WireMsg::Command {
            req_id: req_id.clone(),
            command: command.into(),
            instance_id,
            config,
            note,
        })
        .await
        .map_err(|_| anyhow::anyhow!("Agent is not connected"))?;

        match tokio::time::timeout(std::time::Duration::from_secs(20), rrx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => anyhow::bail!("Response timeout"),
            Err(_) => {
                self.pending.lock().await.remove(&req_id);
                anyhow::bail!("Response timeout")
            }
        }
    }
}
