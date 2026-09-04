mod agent;
mod realm;
mod relay;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "norrna", version = "26.1.14", about = "Norrna agent (Realm kernel)")]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// start encrypted API server or connect as agent
    Api {
        /// Configuration file for global settings (log, dns, network)
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Port to bind the API server (passive mode)
        #[arg(long)]
        port: Option<u16>,
        /// Management server address to connect (active/agent mode)
        #[arg(long)]
        server: Option<String>,
        /// API Key for authentication
        #[arg(long, env = "NORRNA_API_KEY")]
        key: Option<String>,
        /// Agent name
        #[arg(long)]
        name: Option<String>,
    },
    /// convert your legacy configuration into an advanced one
    Convert,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Commands::Convert => {
            anyhow::bail!("attention: you are using a legacy config file!");
        }
        Commands::Api {
            config,
            port,
            server,
            key,
            name,
        } => {
            let key = key.ok_or_else(|| anyhow::anyhow!("Error: API key is required"))?;
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let data = config
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or(cwd);
            if let Some(server) = server {
                agent::run_agent(&server, &key, name, data, config).await?;
            } else if port.is_some() {
                anyhow::bail!("passive API server mode is not used by Norrna-Manager; use --server");
            } else {
                anyhow::bail!("Error: Either --port or --server must be specified\nExamples:\n  Server mode: norrna api --port 9000 --key mykey\n  Agent mode:  norrna api --server 127.0.0.1:3001 --key mykey --name myagent");
            }
        }
    }
    Ok(())
}
