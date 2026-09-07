use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "dododook/Norrna";

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag: String,
    pub notes: String,
    pub html_url: String,
    pub assets: HashMap<String, String>,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub async fn fetch_latest() -> Result<ReleaseInfo> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("norrna-manager/{}", current_version()))
        .build()?;
    let rel: GhRelease = client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = rel.tag_name.trim_start_matches('v').to_string();
    let mut assets = HashMap::new();
    for a in rel.assets {
        assets.insert(a.name, a.browser_download_url);
    }
    Ok(ReleaseInfo {
        version,
        tag: rel.tag_name,
        notes: rel.body,
        html_url: rel.html_url,
        assets,
    })
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    parse_ver(latest) > parse_ver(current)
}

fn parse_ver(s: &str) -> (u64, u64, u64) {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let a = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let b = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let c = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (a, b, c)
}

async fn replace_bin(client: &reqwest::Client, url: &str, dest: &Path) -> Result<()> {
    tracing::info!("downloading {url} -> {}", dest.display());
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() < 1024 {
        anyhow::bail!("downloaded file too small ({} bytes)", bytes.len());
    }
    let tmp = dest.with_extension("new");
    tokio::fs::write(&tmp, &bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
    }
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

pub async fn apply_manager() -> Result<String> {
    let rel = fetch_latest().await?;
    if !is_newer(&rel.version, current_version()) {
        anyhow::bail!("已是最新版本 {}", current_version());
    }
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/norrna-manager"));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!("norrna-manager/{}", current_version()))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let mgr_url = rel
        .assets
        .get("norrna-manager")
        .ok_or_else(|| anyhow::anyhow!("release 里没有 norrna-manager"))?;
    replace_bin(&client, mgr_url, &exe).await?;
    if let Some(url) = rel.assets.get("norrna") {
        let _ = replace_bin(&client, url, &dir.join("norrna")).await;
    }
    schedule_restart("norrna-manager");
    Ok(rel.version)
}

pub fn schedule_restart(service: &str) {
    let service = service.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        let _ = std::process::Command::new("systemctl")
            .args(["restart", &service])
            .status();
    });
}
