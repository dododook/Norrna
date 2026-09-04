use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use norrna_proto::{
    decode_mux_header, decode_udp_mux, encode_mux_header, encode_udp_mux, parse_socket_addr,
    Instance, InstanceConfig,
};

pub struct Running {
    pub instance: Instance,
    abort: JoinHandle<()>,
}

impl Running {
    pub fn abort(self) {
        self.abort.abort();
    }
}

pub async fn spawn_instance(inst: Instance) -> Result<Running> {
    let cfg = inst.config.clone();
    let listen = parse_socket_addr(&cfg.listen)?;
    let handle = tokio::spawn(async move {
        if let Err(e) = run_endpoint(cfg, listen).await {
            tracing::error!("endpoint {listen} stopped: {e}");
        }
    });
    Ok(Running {
        instance: inst,
        abort: handle,
    })
}

async fn run_endpoint(cfg: InstanceConfig, listen: SocketAddr) -> Result<()> {
    let tcp = tokio::spawn(run_tcp(cfg.clone(), listen));
    let udp = tokio::spawn(run_udp(cfg, listen));
    let _ = tokio::join!(tcp, udp);
    Ok(())
}

async fn run_tcp(cfg: InstanceConfig, listen: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!("[tcp] listening on {listen} multiplex={}", cfg.multiplex_mode);
    loop {
        let (mut inbound, peer) = listener.accept().await?;
        let cfg = cfg.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_tcp(&cfg, &mut inbound, peer).await {
                tracing::debug!("[tcp] {peer} {e}");
            }
        });
    }
}

async fn handle_tcp(cfg: &InstanceConfig, inbound: &mut TcpStream, _peer: SocketAddr) -> Result<()> {
    let _ = inbound.set_nodelay(true);
    match cfg.multiplex_mode {
        1 => {
            // server: try mux header then fallback remote
            let mut peek = vec![0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(3), inbound.read(&mut peek)).await??;
            peek.truncate(n);
            if let Some((owner, target, used)) = decode_mux_header(&peek) {
                if let Some(expect) = &cfg.owner_user_id {
                    if expect != &owner {
                        anyhow::bail!("owner mismatch");
                    }
                }
                let rest = peek[used..].to_vec();
                let dest = parse_socket_addr(&target)?;
                let mut outbound = TcpStream::connect(dest).await?;
                let _ = outbound.set_nodelay(true);
                if !rest.is_empty() {
                    outbound.write_all(&rest).await?;
                }
                tokio::io::copy_bidirectional(inbound, &mut outbound).await?;
            } else {
                let remote = cfg.remote.as_str();
                if remote.is_empty() || remote == "127.0.0.1:1" {
                    anyhow::bail!("no default remote");
                }
                let dest = parse_socket_addr(remote)?;
                let mut outbound = TcpStream::connect(dest).await?;
                let _ = outbound.set_nodelay(true);
                if !peek.is_empty() {
                    outbound.write_all(&peek).await?;
                }
                tokio::io::copy_bidirectional(inbound, &mut outbound).await?;
            }
        }
        2 => {
            let ix = parse_socket_addr(&cfg.remote)?;
            let target = cfg
                .final_target
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Client mode requires final_target"))?;
            let owner = cfg
                .owner_user_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Client mode requires owner_user_id"))?;
            let mut outbound = TcpStream::connect(ix).await?;
            let _ = outbound.set_nodelay(true);
            outbound
                .write_all(&encode_mux_header(&owner, &target))
                .await?;
            tokio::io::copy_bidirectional(inbound, &mut outbound).await?;
        }
        _ => {
            let dest = parse_socket_addr(&cfg.remote)?;
            let mut outbound = TcpStream::connect(dest).await?;
            let _ = outbound.set_nodelay(true);
            tokio::io::copy_bidirectional(inbound, &mut outbound).await?;
        }
    }
    Ok(())
}

async fn run_udp(cfg: InstanceConfig, listen: SocketAddr) -> Result<()> {
    let sock = Arc::new(UdpSocket::bind(listen).await?);
    tracing::info!("[udp] listening on {listen}");
    let sessions: Arc<Mutex<HashMap<SocketAddr, (Arc<UdpSocket>, Instant)>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut buf = vec![0u8; 65535];
    loop {
        let (n, src) = sock.recv_from(&mut buf).await?;
        let pkt = &buf[..n];
        match cfg.multiplex_mode {
            1 => {
                if let Some((_owner, target, payload)) = decode_udp_mux(pkt) {
                    if let Ok(dest) = parse_socket_addr(&target) {
                        udp_nat(sock.clone(), sessions.clone(), src, dest, payload).await;
                    }
                } else if cfg.remote != "127.0.0.1:1" {
                    if let Ok(dest) = parse_socket_addr(&cfg.remote) {
                        udp_nat(sock.clone(), sessions.clone(), src, dest, pkt).await;
                    }
                }
            }
            2 => {
                if let (Ok(ix), Some(target), Some(owner)) = (
                    parse_socket_addr(&cfg.remote),
                    cfg.final_target.as_deref(),
                    cfg.owner_user_id.as_deref(),
                ) {
                    let wrapped = encode_udp_mux(owner, target, pkt);
                    udp_nat(sock.clone(), sessions.clone(), src, ix, &wrapped).await;
                }
            }
            _ => {
                if let Ok(dest) = parse_socket_addr(&cfg.remote) {
                    udp_nat(sock.clone(), sessions.clone(), src, dest, pkt).await;
                }
            }
        }
    }
}

async fn udp_nat(
    listen: Arc<UdpSocket>,
    sessions: Arc<Mutex<HashMap<SocketAddr, (Arc<UdpSocket>, Instant)>>>,
    client: SocketAddr,
    remote: SocketAddr,
    payload: &[u8],
) {
    let sock = {
        let mut g = sessions.lock().await;
        if let Some((s, t)) = g.get_mut(&client) {
            *t = Instant::now();
            s.clone()
        } else {
            let s = match UdpSocket::bind("0.0.0.0:0").await {
                Ok(s) => Arc::new(s),
                Err(_) => return,
            };
            let s2 = s.clone();
            let listen2 = listen.clone();
            let sessions2 = sessions.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65535];
                loop {
                    match tokio::time::timeout(Duration::from_secs(60), s2.recv_from(&mut buf)).await {
                        Ok(Ok((n, _))) => {
                            let _ = listen2.send_to(&buf[..n], client).await;
                        }
                        _ => break,
                    }
                }
                sessions2.lock().await.remove(&client);
            });
            g.insert(client, (s.clone(), Instant::now()));
            s
        }
    };
    let _ = sock.send_to(payload, remote).await;
}
