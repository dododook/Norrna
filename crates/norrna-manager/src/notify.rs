use crate::models::AppSettings;
use crate::storage::Storage;
use std::sync::Arc;
use std::time::Duration;

pub async fn send_telegram(settings: &AppSettings, text: &str) -> Result<(), String> {
    if !settings.telegram_enabled {
        return Err("Telegram 通知未开启".into());
    }
    if settings.telegram_bot_token.is_empty() || settings.telegram_chat_id.is_empty() {
        return Err("未配置 Bot Token 或 Chat ID".into());
    }
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        settings.telegram_bot_token.trim()
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "chat_id": settings.telegram_chat_id.trim(),
            "text": text,
            "disable_web_page_preview": true,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Telegram API: {body}"));
    }
    Ok(())
}

pub fn spawn_watch(storage: Arc<Storage>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let settings = storage.settings().await;
            if !settings.telegram_enabled || !settings.notify_offline {
                continue;
            }
            let agents = storage.agents().await;
            let now = chrono::Utc::now();
            for a in agents {
                if a.last_seen.is_empty() {
                    continue;
                }
                let online = a.status == "online";
                let stale = if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&a.last_seen) {
                    now.signed_duration_since(t).num_seconds() > 45
                } else {
                    true
                };
                if !online || stale {
                    if !a.offline_notified {
                        let msg = format!(
                            "Norrna 告警\nAgent 离线: {}\nIP: {}\n时间: {}",
                            a.name,
                            a.ip,
                            now.to_rfc3339()
                        );
                        if send_telegram(&settings, &msg).await.is_ok() {
                            let _ = storage
                                .update_agent(&a.id, |x| x.offline_notified = true)
                                .await;
                        }
                    }
                }
            }
        }
    });
}
