use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use norrna_proto::InstanceConfig;

struct RealmChild {
    child: Child,
    config_path: PathBuf,
    pid_path: PathBuf,
}

/// Official zhboner/realm used as the TCP/UDP forwarding kernel.
/// One process per instance so start/stop of one forward does not bounce the others.
pub struct RealmEngine {
    binary: Option<PathBuf>,
    run_dir: PathBuf,
    children: Mutex<HashMap<String, RealmChild>>,
}

impl RealmEngine {
    pub fn discover(data_dir: &Path) -> Self {
        let binary = find_realm(data_dir);
        match &binary {
            Some(p) => tracing::info!("[realm] kernel binary: {}", p.display()),
            None => tracing::warn!(
                "[realm] official realm not found; install from https://github.com/zhboner/realm/releases/tag/v2.9.6 (or put `realm` in {})",
                data_dir.display()
            ),
        }
        let run_dir = data_dir.join("realm-run");
        let _ = std::fs::create_dir_all(&run_dir);
        Self {
            binary,
            run_dir,
            children: Mutex::new(HashMap::new()),
        }
    }

    pub fn binary_path(&self) -> Option<&Path> {
        self.binary.as_deref()
    }

    pub async fn start_one(&self, id: &str, cfg: &InstanceConfig, global: &Value) -> Result<()> {
        self.stop_one(id).await;
        let Some(bin) = self.binary.clone() else {
            anyhow::bail!(
                "official realm binary not found. Place `realm` in the agent directory or PATH. Download: https://github.com/zhboner/realm/releases/tag/v2.9.6"
            );
        };
        let mut doc = global.clone();
        if !doc.is_object() {
            doc = json!({});
        }
        if doc.get("log").is_none() {
            doc["log"] = json!({ "level": "info", "output": "stdout" });
        }
        doc["endpoints"] = json!([endpoint_from(cfg)]);
        let config_path = self.run_dir.join(format!("{id}.json"));
        let pid_path = self.run_dir.join(format!("{id}.pid"));
        let tmp = config_path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(&doc)?).await?;
        tokio::fs::rename(&tmp, &config_path).await?;
        kill_stale_file(&pid_path).await;
        #[cfg(unix)]
        {
            let needle = config_path.to_string_lossy().into_owned();
            let _ = Command::new("pkill").args(["-f", &needle]).status().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let mut cmd = Command::new(&bin);
        cmd.arg("-c")
            .arg(&config_path)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn realm ({}): {e}", bin.display()))?;
        if let Some(pid) = child.id() {
            let _ = tokio::fs::write(&pid_path, pid.to_string()).await;
            tracing::info!("[realm] {id} started pid={pid} -c {}", config_path.display());
        }
        self.children.lock().await.insert(
            id.to_string(),
            RealmChild {
                child,
                config_path,
                pid_path,
            },
        );
        tokio::time::sleep(Duration::from_millis(350)).await;
        let mut g = self.children.lock().await;
        if let Some(c) = g.get_mut(id) {
            if let Ok(Some(status)) = c.child.try_wait() {
                g.remove(id);
                anyhow::bail!("realm exited immediately ({status}). Check listen addresses and that ports are free.");
            }
        }
        Ok(())
    }

    pub async fn stop_one(&self, id: &str) {
        if let Some(mut c) = self.children.lock().await.remove(id) {
            let _ = c.child.kill().await;
            let _ = c.child.wait().await;
            let _ = tokio::fs::remove_file(&c.pid_path).await;
            let _ = tokio::fs::remove_file(&c.config_path).await;
        }
    }

    pub async fn respawn_if_dead(&self) {
        let Some(bin) = self.binary.clone() else {
            return;
        };
        let mut g = self.children.lock().await;
        let mut dead = Vec::new();
        for (id, c) in g.iter_mut() {
            if matches!(c.child.try_wait(), Ok(Some(_))) {
                dead.push(id.clone());
            }
        }
        for id in dead {
            tracing::warn!("[realm] {id} exited unexpectedly, restarting");
            let Some(old) = g.remove(&id) else {
                continue;
            };
            if !old.config_path.is_file() {
                continue;
            }
            let mut cmd = Command::new(&bin);
            cmd.arg("-c")
                .arg(&old.config_path)
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            match cmd.spawn() {
                Ok(child) => {
                    if let Some(pid) = child.id() {
                        let _ = std::fs::write(&old.pid_path, pid.to_string());
                    }
                    g.insert(
                        id,
                        RealmChild {
                            child,
                            config_path: old.config_path,
                            pid_path: old.pid_path,
                        },
                    );
                }
                Err(e) => tracing::error!("[realm] restart {id} failed: {e}"),
            }
        }
    }
}

async fn kill_stale_file(pid_path: &Path) {
    let Ok(s) = tokio::fs::read_to_string(pid_path).await else {
        return;
    };
    let Ok(pid) = s.trim().parse::<u32>() else {
        return;
    };
    tracing::info!("[realm] killing stale pid {pid}");
    kill_pid(pid);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = tokio::fs::remove_file(pid_path).await;
}

pub fn endpoint_from(cfg: &InstanceConfig) -> Value {
    let mut ep = json!({
        "listen": to_realm_addr(&cfg.listen),
        "remote": to_realm_addr(&cfg.remote),
    });
    if !cfg.extra_remotes.is_empty() {
        ep["extra_remotes"] = json!(
            cfg.extra_remotes
                .iter()
                .map(|s| to_realm_addr(s))
                .collect::<Vec<_>>()
        );
    }
    if let Some(net) = &cfg.network {
        ep["network"] = net.clone();
    }
    ep
}

pub fn load_global(conf_path: &Path) -> Value {
    let mut out = json!({
        "log": { "level": "info", "output": "stdout" },
        "network": { "no_tcp": false, "use_udp": true }
    });
    if let Ok(raw) = std::fs::read(conf_path) {
        if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&raw) {
            if let Some(v) = map.get("log") {
                out["log"] = v.clone();
            }
            if let Some(v) = map.get("dns") {
                out["dns"] = v.clone();
            }
            if let Some(v) = map.get("network") {
                out["network"] = v.clone();
            }
        }
    }
    out
}

/// Convert Zelay/Norrna listen strings (`:::443`, `*:80`) to Realm form.
pub fn to_realm_addr(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() || s.starts_with('[') {
        return s.to_string();
    }
    if let Some(port) = s.strip_prefix(":::") {
        return format!("[::]:{port}");
    }
    if let Some(rest) = s.strip_prefix("*:") {
        return format!("0.0.0.0:{rest}");
    }
    let colons = s.chars().filter(|c| *c == ':').count();
    if colons > 1 {
        if let Some(idx) = s.rfind(':') {
            let host = &s[..idx];
            let port = &s[idx + 1..];
            if !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit())
                && host.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
            {
                return format!("[{host}]:{port}");
            }
        }
    }
    s.to_string()
}

pub fn find_realm(data_dir: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NORRNA_REALM") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut cands = vec![
        data_dir.join("realm"),
        data_dir.join("realm.exe"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("realm"));
            cands.push(dir.join("realm.exe"));
        }
    }
    cands.push(PathBuf::from("/usr/local/bin/realm"));
    cands.push(PathBuf::from("/usr/bin/realm"));
    for p in cands {
        if p.is_file() {
            return Some(p);
        }
    }
    find_in_path("realm")
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
        #[cfg(windows)]
        {
            let p = dir.join(format!("{name}.exe"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        std::thread::sleep(Duration::from_millis(150));
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_forms() {
        assert_eq!(to_realm_addr(":::443"), "[::]:443");
        assert_eq!(to_realm_addr("0.0.0.0:80"), "0.0.0.0:80");
        assert_eq!(to_realm_addr("*:80"), "0.0.0.0:80");
        assert_eq!(to_realm_addr("[::]:80"), "[::]:80");
        assert_eq!(to_realm_addr("example.com:443"), "example.com:443");
        assert_eq!(to_realm_addr("1.1.1.1:443"), "1.1.1.1:443");
        assert_eq!(to_realm_addr("::1:443"), "[::1]:443");
    }
}
