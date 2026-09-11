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

fn bin_names(stem: &str) -> Vec<String> {
    match std::env::consts::ARCH {
        "aarch64" => vec![format!("{stem}-linux-arm64")],
        "x86_64" => vec![format!("{stem}-linux-amd64"), stem.to_string()],
        _ => vec![stem.to_string()],
    }
}

fn pick_asset<'a>(assets: &'a [GhAsset], stem: &str) -> Option<&'a str> {
    for name in bin_names(stem) {
        if let Some(a) = assets.iter().find(|x| x.name == name) {
            return Some(a.browser_download_url.as_str());
        }
    }
    None
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
    let norrna_url = pick_asset(&rel.assets, "norrna")
        .ok_or_else(|| anyhow::anyhow!("release 里没有当前架构的 norrna"))?;
    replace_bin(&client, norrna_url, &exe).await?;
    if let Err(e) = apply_realm().await {
        tracing::warn!("[update] official realm: {e}");
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(2));
        let _ = std::process::Command::new("systemctl")
            .args(["restart", "norrna-agent"])
            .status();
    });
    Ok(version)
}

pub fn realm_dest(data_hint: Option<&Path>) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("realm");
            if p.exists() || data_hint.is_none() {
                return p;
            }
        }
    }
    if let Some(d) = data_hint {
        let p = d.join("realm");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("/etc/norrna/realm")
}

fn linux_realm_assets() -> Vec<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => vec![
            "realm-x86_64-unknown-linux-musl.tar.gz",
            "realm-x86_64-unknown-linux-gnu.tar.gz",
        ],
        "aarch64" => vec![
            "realm-aarch64-unknown-linux-musl.tar.gz",
            "realm-aarch64-unknown-linux-gnu.tar.gz",
        ],
        _ => vec![],
    }
}

/// Download latest official Realm from zhboner/realm and replace the local binary.
pub async fn apply_realm() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!("norrna/{}", current_version()))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let rel: GhRelease = client
        .get("https://api.github.com/repos/zhboner/realm/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = rel.tag_name.clone();
    let assets = linux_realm_assets();
    if assets.is_empty() {
        anyhow::bail!("当前架构 {} 没有官方 Realm 包", std::env::consts::ARCH);
    }
    let mut tar_bytes = None;
    let mut used = String::new();
    for name in assets {
        if let Some(a) = rel.assets.iter().find(|x| x.name == name) {
            tracing::info!("[update] realm {}", a.browser_download_url);
            match client.get(&a.browser_download_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(b) = resp.bytes().await {
                        if b.len() > 1024 {
                            tar_bytes = Some(b);
                            used = name.to_string();
                            break;
                        }
                    }
                }
                _ => continue,
            }
        }
    }
    let tar_bytes = tar_bytes.ok_or_else(|| anyhow::anyhow!("无法下载官方 Realm {version}"))?;
    let tmpdir = std::env::temp_dir().join(format!("norrna-realm-{}", std::process::id()));
    tokio::fs::create_dir_all(&tmpdir).await?;
    let tar_path = tmpdir.join("realm.tar.gz");
    tokio::fs::write(&tar_path, &tar_bytes).await?;
    let status = tokio::process::Command::new("tar")
        .args(["-xzf"])
        .arg(&tar_path)
        .current_dir(&tmpdir)
        .status()
        .await?;
    if !status.success() {
        let _ = tokio::fs::remove_dir_all(&tmpdir).await;
        anyhow::bail!("解压 Realm 失败 ({used})");
    }
    let extracted = if tmpdir.join("realm").is_file() {
        tmpdir.join("realm")
    } else {
        let _ = tokio::fs::remove_dir_all(&tmpdir).await;
        anyhow::bail!("压缩包里没有 realm 可执行文件");
    };
    let dest = realm_dest(None);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755)).await?;
    }
    tokio::fs::copy(&extracted, &dest).await?;
    let _ = tokio::fs::remove_dir_all(&tmpdir).await;
    tracing::info!("[update] realm {} installed to {}", version, dest.display());
    Ok(version)
}
