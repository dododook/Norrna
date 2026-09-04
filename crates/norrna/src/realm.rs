use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use norrna_proto::InstanceConfig;

/// Official zhboner/realm used as the TCP/UDP forwarding kernel.
pub struct RealmEngine {
    binary: Option<PathBuf>,
    config_path: PathBuf,
    pid_path: PathBuf,
    child: Mutex<Option<Child>>,
    wanted: AtomicBool,
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
        Self {
            binary,
            config_path: data_dir.join("realm-runtime.json"),
            pid_path: data_dir.join("realm.pid"),
            child: Mutex::new(None),
            wanted: AtomicBool::new(false),
        }
    }

    pub fn binary_path(&self) -> Option<&Path> {
        self.binary.as_deref()
    }

    pub async fn sync(&self, endpoints: &[Value], global: &Value) -> Result<()> {
        if endpoints.is_empty() {
            self.wanted.store(false, Ordering::SeqCst);
            self.stop_child().await;
            let _ = tokio::fs::remove_file(&self.config_path).await;
            return Ok(());
        }
        let Some(bin) = self.binary.clone() else {
            anyhow::bail!(
                "official realm binary not found. Place `realm` in the agent directory or PATH. Download: https://github.com/zhboner/realm/releases/tag/v2.9.6"
            );
        };

        let mut cfg = global.clone();
        if !cfg.is_object() {
            cfg = json!({});
        }
        if cfg.get("log").is_none() {
            cfg["log"] = json!({ "level": "info", "output": "stdout" });
        }
        cfg["endpoints"] = Value::Array(endpoints.to_vec());

        let body = serde_json::to_vec_pretty(&cfg)?;
        let tmp = self.config_path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &body).await?;
        tokio::fs::rename(&tmp, &self.config_path).await?;
        tracing::info!(
            "[realm] wrote {} endpoint(s) -> {}",
            endpoints.len(),
            self.config_path.display()
        );

        self.wanted.store(true, Ordering::SeqCst);
        self.restart(&bin).await
    }

    pub async fn respawn_if_dead(&self) {
        if !self.wanted.load(Ordering::SeqCst) {
            return;
        }
        let Some(bin) = self.binary.clone() else {
            return;
        };
        let mut g = self.child.lock().await;
        let dead = match g.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => true,
        };
        if !dead {
            return;
        }
        tracing::warn!("[realm] process exited unexpectedly, restarting");
        *g = None;
        drop(g);
        if let Err(e) = self.restart(&bin).await {
            tracing::error!("[realm] restart failed: {e}");
        }
    }

    async fn restart(&self, bin: &Path) -> Result<()> {
        self.stop_child().await;
        self.kill_stale_pid().await;
        #[cfg(unix)]
        {
            let needle = self.config_path.to_string_lossy().into_owned();
            let _ = Command::new("pkill").args(["-f", &needle]).status().await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let mut cmd = Command::new(bin);
        cmd.arg("-c")
            .arg(&self.config_path)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn realm ({}): {e}", bin.display()))?;
        if let Some(pid) = child.id() {
            let _ = tokio::fs::write(&self.pid_path, pid.to_string()).await;
            tracing::info!("[realm] started pid={pid} -c {}", self.config_path.display());
        }
        {
            let mut g = self.child.lock().await;
            *g = Some(child);
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut g = self.child.lock().await;
        if let Some(c) = g.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                *g = None;
                anyhow::bail!("realm exited immediately ({status}). Check listen addresses and that ports are free.");
            }
        }
        Ok(())
    }

    async fn stop_child(&self) {
        let mut g = self.child.lock().await;
        if let Some(mut c) = g.take() {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
        let _ = tokio::fs::remove_file(&self.pid_path).await;
    }

    async fn kill_stale_pid(&self) {
        let Ok(s) = tokio::fs::read_to_string(&self.pid_path).await else {
            return;
        };
        let Ok(pid) = s.trim().parse::<u32>() else {
            return;
        };
        tracing::info!("[realm] killing stale pid {pid}");
        kill_pid(pid);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tokio::fs::remove_file(&self.pid_path).await;
    }
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

fn find_realm(data_dir: &Path) -> Option<PathBuf> {
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
