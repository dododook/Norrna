mod agent;
mod realm;
mod relay;
mod unlock;
mod updater;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "norrna", version = "26.1.34", about = "Norrna agent (Realm kernel)")]
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
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if let Some(bin) = crate::realm::find_realm(&cwd) {
                let st = std::process::Command::new(bin).arg("convert").status()?;
                std::process::exit(st.code().unwrap_or(1));
            }
            anyhow::bail!("attention: you are using a legacy config file!");
        }
        Commands::Api {
            config,
            port,
            server,
            key,
            name,
        } => {
            let key = key
                .or_else(|| std::env::var("REALM_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("Error: API key is required\nProvide it via --key argument or NORRNA_API_KEY / REALM_API_KEY environment variable")
                })?;
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let data = config
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or(cwd);
            if let Some(server) = server {
                agent::run_agent(&server, &key, name, data, config).await?;
            } else if let Some(port) = port {
                agent::run_passive(port, &key, name, data, config).await?;
            } else {
                anyhow::bail!("Error: Either --port or --server must be specified\nExamples:\n  Server mode: norrna api --port 9000 --key mykey\n  Agent mode:  norrna api --server 127.0.0.1:3001 --key mykey --name myagent");
            }
        }
    }
    Ok(())
}
