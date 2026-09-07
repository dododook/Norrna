mod auth;
mod hub;
mod models;
mod notify;
mod storage;
mod updater;
mod web;

use clap::Parser;
use hub::AgentHub;
use std::sync::Arc;
use storage::Storage;
use tracing_subscriber::EnvFilter;
use web::{router, AppState};

#[derive(Parser, Debug)]
#[command(name = "norrna-manager", version = "26.1.25", about = "Norrna-Manager 26.1.25")]
struct Args {
    /// Web 管理面板端口
    #[arg(long, env = "WEBPORT")]
    webport: u16,
    /// Agent TCP server port
    #[arg(long, env = "AGENTPORT")]
    agentport: u16,
    /// Data directory
    #[arg(long = "data-dir", env = "DATA_DIR")]
    data_dir: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let data_dir = std::path::PathBuf::from(&args.data_dir);
    let storage = Arc::new(Storage::open(&data_dir).await?);
    let hub = AgentHub::new(storage.clone());

    let agent_bind: std::net::SocketAddr = format!("[::]:{}", args.agentport).parse()?;
    let web_bind: std::net::SocketAddr = format!("[::]:{}", args.webport).parse()?;

    notify::spawn_watch(storage.clone());
    let hub_clone = hub.clone();
    tokio::spawn(async move {
        if let Err(e) = hub_clone.serve(agent_bind).await {
            tracing::error!("Agent TCP server error: {e}");
        }
    });

    println!("========================================");
    println!("Norrna-Manager 26.1.25");
    println!("HTTP: http://0.0.0.0:{}", args.webport);
    println!("Login: http://0.0.0.0:{}/login", args.webport);
    println!("Agent Port: {}", args.agentport);
    println!("Data: {}", data_dir.display());
    println!("========================================");

    let app = router(AppState {
        storage,
        hub,
        agent_port: args.agentport,
    });
    let listener = tokio::net::TcpListener::bind(web_bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
