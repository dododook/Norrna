use crate::models::{AgentConfig, AgentsData, ServerConfig, ServersData, User, UsersData};
use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

pub struct Storage {
    dir: PathBuf,
    users: RwLock<UsersData>,
    agents: RwLock<AgentsData>,
    servers: RwLock<ServersData>,
}

impl Storage {
    pub async fn open(dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(dir).await?;
        let users = load_or(dir.join("users.json"), UsersData { users: vec![] }).await?;
        let agents = load_or(dir.join("agents.json"), AgentsData { agents: vec![] }).await?;
        let servers = load_or(dir.join("servers.json"), ServersData { servers: vec![] }).await?;
        Ok(Self {
            dir: dir.to_path_buf(),
            users: RwLock::new(users),
            agents: RwLock::new(agents),
            servers: RwLock::new(servers),
        })
    }

    pub async fn users(&self) -> Vec<User> {
        self.users.read().await.users.clone()
    }

    pub async fn save_users(&self, users: Vec<User>) -> Result<()> {
        let data = UsersData { users };
        save(&self.dir.join("users.json"), &data).await?;
        *self.users.write().await = data;
        Ok(())
    }

    pub async fn agents(&self) -> Vec<AgentConfig> {
        self.agents.read().await.agents.clone()
    }

    pub async fn save_agents(&self, agents: Vec<AgentConfig>) -> Result<()> {
        let data = AgentsData { agents };
        save(&self.dir.join("agents.json"), &data).await?;
        *self.agents.write().await = data;
        Ok(())
    }

    pub async fn update_agent<F>(&self, id: &str, f: F) -> Result<Option<AgentConfig>>
    where
        F: FnOnce(&mut AgentConfig),
    {
        let mut data = self.agents.write().await;
        if let Some(a) = data.agents.iter_mut().find(|a| a.id == id) {
            f(a);
            let cloned = a.clone();
            save(&self.dir.join("agents.json"), &*data).await?;
            return Ok(Some(cloned));
        }
        Ok(None)
    }

    pub async fn servers(&self) -> Vec<ServerConfig> {
        self.servers.read().await.servers.clone()
    }

    pub async fn save_servers(&self, servers: Vec<ServerConfig>) -> Result<()> {
        let data = ServersData { servers };
        save(&self.dir.join("servers.json"), &data).await?;
        *self.servers.write().await = data;
        Ok(())
    }
}

async fn load_or<T: serde::de::DeserializeOwned>(path: PathBuf, default: T) -> Result<T> {
    match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::warn!("Error loading {}: {e}. Deleting corrupted file...", path.display());
                let _ = tokio::fs::remove_file(&path).await;
                Ok(default)
            }
        },
        Err(_) => Ok(default),
    }
}

async fn save<T: serde::Serialize>(path: &Path, v: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(v)?).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}
