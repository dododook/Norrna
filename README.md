# Norrna

轻量级转发面板：Web 管理多节点 Agent，创建 TCP/UDP 转发与端口复用。

## 编译

需要 Rust（建议 1.80+）：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo build --release
```

产物：

- `target/release/norrna-manager` 面板
- `target/release/norrna` Agent / 转发内核

## 安装面板

```bash
bash scripts/norrna_manager.sh webport=3000 agentport=3001
```

默认：

| 项 | 路径 / 端口 |
|---|---|
| 程序 | `/etc/norrna-manager/norrna-manager` |
| 数据 | `/etc/norrna-manager/data` |
| Web | `http://IP:3000` |
| Agent 端口 | `3001` |

打开面板创建管理员，再添加 Agent，复制部署命令。

## 安装 Agent

在节点上执行面板给出的命令，或：

```bash
bash scripts/norrna_agent.sh server=面板IP:3001 apikey=你的KEY
```

Agent 安装到 `/etc/norrna`，配置为 `/etc/norrna/norrna.conf`。

## 手动启动（不装 systemd）

```bash
./norrna-manager --webport 3000 --agentport 3001 --data-dir ./data
./norrna api --server 127.0.0.1:3001 --key YOUR_KEY
```

## 端口复用

| multiplex_mode | 含义 |
|---|---|
| 0 | 普通转发 listen ↔ remote |
| 1 | 服务端，入站 `NORRNAMX` 头转到最终目标 |
| 2 | 客户端，连到复用口并带上 `final_target` |

UDP 魔数：`NORRNAUDP`。

## 目录

```
crates/norrna-manager/  面板
crates/norrna/          Agent + 转发
crates/norrna-proto/    控制协议
scripts/                一键安装与 systemd
```
