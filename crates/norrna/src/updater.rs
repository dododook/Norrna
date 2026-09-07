use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "dododook/Norrna";

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

async fn replace_bin(client: &reqwest::Client, url: &str, dest: &Path) -> Result<()> {
    tracing::info!("[update] {url} -> {}", dest.display());
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() < 1024 {
        anyhow::bail!("downloaded file too small");
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

pub async fn apply_agent() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!("norrna/{}", current_version()))
        .redirect(reqwest::redirect::Policy::limited(10))
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
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/norrna"));
    let norrna_url = rel
        .assets
        .iter()
        .find(|a| a.name == "norrna")
        .map(|a| a.browser_download_url.as_str())
        .ok_or_else(|| anyhow::anyhow!("release 里没有 norrna"))?;
    replace_bin(&client, norrna_url, &exe).await?;
    if let Some(url) = rel
        .assets
        .iter()
        .find(|a| a.name == "realm")
        .map(|a| a.browser_download_url.clone())
    {
        let realm = if dir.join("realm").exists() {
            dir.join("realm")
        } else {
            PathBuf::from("/etc/norrna/realm")
        };
        let _ = replace_bin(&client, &url, &realm).await;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(2));
        let _ = std::process::Command::new("systemctl")
            .args(["restart", "norrna-agent"])
            .status();
    });
    Ok(version)
}
