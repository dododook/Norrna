# Norrna

轻量级转发面板：Web 管理多节点 Agent。普通 TCP/UDP 转发由官方 [Realm](https://github.com/zhboner/realm)（v2.9.6）完成，端口复用由 Agent 叠加。

## 编译

需要 Rust（建议 1.80+）：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo build --release
```

产物：

- `target/release/norrna-manager` 面板
- `target/release/norrna` Agent（调度 Realm / 端口复用）

Agent 安装脚本会再下载官方 `realm` 到 `/etc/norrna/realm`。

## 安装面板

有 GitHub Release 后，任意机器可直接：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_manager.sh) webport=3000 agentport=3001
```

或在本仓库编译后本地安装：

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

在节点上执行面板给出的命令。脚本默认从本仓库 Release 下载 `norrna`；若尚未发布 Release，则使用本地编译文件。

或：

```bash
bash scripts/norrna_agent.sh server=面板IP:3001 apikey=你的KEY
```

Agent 安装到 `/etc/norrna`：

| 项 | 路径 |
|---|---|
| Agent | `/etc/norrna/norrna` |
| 官方 Realm | `/etc/norrna/realm` |
| 全局配置（DNS/网络） | `/etc/norrna/norrna.conf` |
| Realm 运行配置 | `/etc/norrna/realm-runtime.json` |

## 手动启动（不装 systemd）

```bash
./norrna-manager --webport 3000 --agentport 3001 --data-dir ./data
./norrna api --server 127.0.0.1:3001 --key YOUR_KEY
```

## 转发内核

| multiplex_mode | 内核 | 含义 |
|---|---|---|
| 0 | 官方 Realm | 普通转发 listen ↔ remote（含 UDP、PROXY、WS/TLS 等 Realm 能力，配置写进 `network`） |
| 1 | Norrna overlay | 服务端，入站 `NORRNAMX` 头转到最终目标 |
| 2 | Norrna overlay | 客户端，连到复用口并带上 `final_target` |

UDP 复用魔数：`NORRNAUDP`。官方 Realm 没有 Zelay 那套端口复用协议，所以 1/2 仍由 Agent 自己转。

可执行文件查找顺序：`NORRNA_REALM` → `/etc/norrna/realm` → 与 `norrna` 同目录 → `PATH`。

## 目录

```
crates/norrna-manager/  面板
crates/norrna/          Agent + 转发
crates/norrna-proto/    控制协议
scripts/                一键安装与 systemd
```
