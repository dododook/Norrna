use serde::Serialize;
use std::time::Duration;

#[derive(Serialize)]
pub struct UnlockItem {
    pub name: String,
    pub status: String,
    pub detail: String,
}

pub async fn run_unlock_checks() -> Vec<UnlockItem> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return vec![UnlockItem {
                name: "HTTP".into(),
                status: "failed".into(),
                detail: e.to_string(),
            }]
        }
    };

    let (
        chatgpt,
        netflix,
        youtube,
        disney,
        tiktok,
        spotify,
        google,
        claude,
    ) = tokio::join!(
        check_status(&client, "ChatGPT", "https://android.chat.openai.com/"),
        check_netflix(&client),
        check_status(&client, "YouTube", "https://www.youtube.com/"),
        check_status(&client, "Disney+", "https://www.disneyplus.com/"),
        check_status(&client, "TikTok", "https://www.tiktok.com/"),
        check_status(&client, "Spotify", "https://www.spotify.com/"),
        check_status(&client, "Google", "https://www.google.com/"),
        check_status(&client, "Claude", "https://claude.ai/"),
    );

    vec![chatgpt, netflix, youtube, disney, tiktok, spotify, google, claude]
}

async fn check_status(client: &reqwest::Client, name: &str, url: &str) -> UnlockItem {
    match client.get(url).send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            if code == 403 || code == 451 {
                UnlockItem {
                    name: name.into(),
                    status: "no".into(),
                    detail: format!("HTTP {code}"),
                }
            } else if code >= 200 && code < 400 {
                UnlockItem {
                    name: name.into(),
                    status: "yes".into(),
                    detail: format!("HTTP {code}"),
                }
            } else {
                UnlockItem {
                    name: name.into(),
                    status: "no".into(),
                    detail: format!("HTTP {code}"),
                }
            }
        }
        Err(e) => UnlockItem {
            name: name.into(),
            status: "failed".into(),
            detail: shorten(&e.to_string()),
        },
    }
}

async fn check_netflix(client: &reqwest::Client) -> UnlockItem {
    match client
        .get("https://www.netflix.com/title/80018499")
        .send()
        .await
    {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let url = resp.url().to_string();
            let body = resp.text().await.unwrap_or_default();
            let blocked = code == 404
                || url.contains("/not-available")
                || body.contains("Not Available")
                || body.contains("isn't available");
            if blocked {
                UnlockItem {
                    name: "Netflix".into(),
                    status: "no".into(),
                    detail: format!("HTTP {code}"),
                }
            } else if code >= 200 && code < 400 {
                UnlockItem {
                    name: "Netflix".into(),
                    status: "yes".into(),
                    detail: format!("HTTP {code}"),
                }
            } else {
                UnlockItem {
                    name: "Netflix".into(),
                    status: "no".into(),
                    detail: format!("HTTP {code}"),
                }
            }
        }
        Err(e) => UnlockItem {
            name: "Netflix".into(),
            status: "failed".into(),
            detail: shorten(&e.to_string()),
        },
    }
}

fn shorten(s: &str) -> String {
    let s: String = s.chars().take(80).collect();
    s
}
