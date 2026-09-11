# Norrna

轻量级转发面板：Web 管多节点 Agent，普通 TCP/UDP 走官方 [Realm](https://github.com/zhboner/realm) v2.9.6，端口复用由 Agent 叠加。

- 面板一键安装 / 更新 / 卸载
- 节点复制命令即装，自动拉取 `norrna` 和官方 `realm`
- 每条普通转发独立 Realm 进程，互不影响
- 支持 **linux-amd64** 和 **linux-arm64**（aarch64，例如 Neoverse / 常见 ARM VPS）

仓库：https://github.com/dododook/Norrna

版本号统一为 **26.1.x**，GitHub Release 标签为 `v26.1.x`（例如 `v26.1.20`）。面板页脚、`--version` 和 Release 是同一套。

---

## 快速开始

### 1. 安装面板（root）

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_manager.sh) webport=3000 agentport=3001
```

自定义端口或数据目录：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_manager.sh) \
  webport=8080 agentport=9000 datadir=/data/norrna
```

浏览器打开 `http://服务器IP:3000`，先创建管理员。

脚本会同时把 Agent 二进制放到 `/etc/norrna-manager/norrna`，供面板一键部署下载。

### Docker 安装面板

需要 Docker 和 Compose。镜像支持 **amd64** 和 **arm64**。

```bash
mkdir -p norrna && cd norrna
curl -fsSL -o docker-compose.yml \
  https://raw.githubusercontent.com/dododook/Norrna/main/docker-compose.yml
docker compose up -d
```

不要加 `--build`。那是从源码编译，需要整个仓库里的 `Dockerfile`。

拉镜像若 403，到 GitHub → Packages 把 `norrna` 设成 Public。或本机构建：

```bash
git clone https://github.com/dododook/Norrna.git
cd Norrna
docker compose -f docker-compose.yml -f docker-compose.build.yml up -d --build
```

打开 `http://服务器IP:3000`。数据在 Docker volume `norrna-data`。改端口编辑 compose 里的 `ports` 和 `WEBPORT` / `AGENTPORT`。

Agent 仍装在节点机上（不要装进这个容器）。安全组放行 **3000**（网页）和 **3001**（Agent 接入）。

```bash
docker compose logs -f
docker compose pull && docker compose up -d
```

### 2. 安装 Agent

面板 → 添加 Agent → 复制部署命令，在 **节点机** 上执行，例如：

```bash
bash <(curl -fsSL http://面板IP:3000/norrna_agent.sh) \
  server=面板IP:3001 \
  apikey=你的KEY \
  dns=223.5.5.5:53,119.29.29.29:53
```

- Agent 和面板 **同一台**：把 `server=` 改成 `127.0.0.1:3001`，避免走公网 IP 被安全组拦住。
- 节点在 **另一台**：面板机防火墙 / 云安全组放行 **3001**（和 Web 的 3000 不是同一个口）。

面板里 Agent 变绿后即可创建转发。节点上还要放行你监听的业务端口。

转发列表点 **延迟** 数字即可拨测（「更多」里也有）：由该节点对远程地址做 3 次 TCP 连接，显示平均延迟（悬停可看 min/avg/max 和丢包）。普通转发测 `remote`，端口复用客户端测 `final_target`。

Agent 卡片 **解锁**：测这台机器自己的网卡出口。落地机请把 Agent 装在落地上再点这里。

转发列表 **落地解锁**：仅当远程是 HTTP/SOCKS 代理时可用。普通 TCP 转发（Realm listen→remote）无法经隧道测 Netflix，请在落地安装 Agent 后点「解锁」。

右上角可切换 **浅色 / 深色** 主题（保存在浏览器里）。**通知设置** 可填 Telegram Bot Token 和 Chat ID。Agent 离线、月流量超过编辑里填的限额时会推送到 TG。流量按节点网卡累计，每月自动清零。

---

## 路径和端口

### 面板

| 项 | 默认 |
|---|---|
| 程序 | `/etc/norrna-manager/norrna-manager` |
| 给节点下载的 Agent | `/etc/norrna-manager/norrna` |
| 数据 | `/etc/norrna-manager/data` |
| Web | `http://IP:3000` |
| Agent 端口 | `3001` |
| systemd | `norrna-manager` |
| Docker 镜像 | `ghcr.io/dododook/norrna` |

### Agent

| 项 | 默认 |
|---|---|
| 程序 | `/etc/norrna/norrna` |
| 官方 Realm | `/etc/norrna/realm` |
| 全局配置（DNS/网络） | `/etc/norrna/norrna.conf` |
| 实例数据 | `/etc/norrna/instances/` |
| systemd | `norrna-agent` |

---

## 更新

更新会从 GitHub `releases/latest` 拉最新二进制。面板和节点要成对升级，只换一边会出现连不上（日志里常见 `Decryption failed`）。

### 网页更新（推荐）

面板右上角 **检查更新**：

- 对照当前版本和 GitHub 最新版
- **更新面板**：下载新二进制并重启 `norrna-manager`（页面约 8 秒后刷新）
- 左侧 Agent 卡片 **更新**：给该节点下发 Norrna 更新并重启 Agent
- **更新内核**：从 [zhboner/realm](https://github.com/zhboner/realm/releases) 拉官方最新 Realm，只重启转发进程（Agent 本身不重启）

第一次仍需命令行升到带此功能的版本；之后就可以在网页里升。

### 命令行更新面板

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_manager.sh) --update
```

等价：`update`。脚本会备份旧文件到 `/etc/norrna-manager/norrna-manager.bak`，并刷新旁边的 `norrna`。

手动强制覆盖：

```bash
systemctl stop norrna-manager
curl -fL -o /etc/norrna-manager/norrna-manager https://github.com/dododook/Norrna/releases/latest/download/norrna-manager
curl -fL -o /etc/norrna-manager/norrna https://github.com/dododook/Norrna/releases/latest/download/norrna
chmod +x /etc/norrna-manager/norrna-manager /etc/norrna-manager/norrna
systemctl start norrna-manager
/etc/norrna-manager/norrna-manager --version
```

### 更新 Agent

在 **节点** 上：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_agent.sh) --update
```

`update` / `upgrade` 也可以。会备份 `/etc/norrna/norrna`，并重装官方 Realm。

手动强制覆盖：

```bash
systemctl stop norrna-agent
curl -fL -o /etc/norrna/norrna https://github.com/dododook/Norrna/releases/latest/download/norrna
curl -fL -o /etc/norrna/realm  https://github.com/dododook/Norrna/releases/latest/download/realm
chmod +x /etc/norrna/norrna /etc/norrna/realm
systemctl start norrna-agent
/etc/norrna/norrna --version
/etc/norrna/realm -v
```

回滚面板：

```bash
mv /etc/norrna-manager/norrna-manager.bak /etc/norrna-manager/norrna-manager
systemctl restart norrna-manager
```

---

## 卸载

### 卸载面板

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_manager.sh) --uninstall
```

等价：`uninstall`。需输入 `yes` 确认，会删掉服务和 `/etc/norrna-manager`（含数据）。

### 卸载 Agent

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/dododook/Norrna/main/scripts/norrna_agent.sh) --uninstall
```

`uninstall` / `remove` 也可以。会停服务；是否删除 `/etc/norrna` 配置和实例数据会再问一次。

---

## 日常命令

### 面板

```bash
systemctl start norrna-manager
systemctl stop norrna-manager
systemctl restart norrna-manager
systemctl status norrna-manager
journalctl -u norrna-manager -f
```

### Agent

```bash
systemctl start norrna-agent
systemctl stop norrna-agent
systemctl restart norrna-agent
systemctl status norrna-agent
journalctl -u norrna-agent -f
```

脚本帮助：

```bash
bash norrna_manager.sh --help
bash norrna_agent.sh --help
```

---

## 防火墙

至少放行：

| 端口 | 用途 |
|---|---|
| 3000/tcp（或你设的 webport） | 面板 Web |
| 3001/tcp（或你设的 agentport） | Agent 连面板 |
| 转发监听端口 | 业务流量 |

示例（ufw）：

```bash
ufw allow 3000/tcp
ufw allow 3001/tcp
```

---

## 转发说明

| multiplex_mode | 内核 | 含义 |
|---|---|---|
| 0 | 官方 Realm | 普通 listen ↔ remote，每条一个进程 |
| 1 | overlay | 端口复用服务端（MySQL 握手伪装 `5.7.44-realm`） |
| 2 | overlay | 端口复用客户端，带 `final_target` |

UDP 复用魔数：`ZELAY_UDP`（兼容 `NORRNAUDP`）。

被动模式（节点先监听，面板「服务器」去连）：

```bash
/etc/norrna/norrna api -c /etc/norrna/norrna.conf --port 9000 --key 你的KEY
```

Realm 查找顺序：`NORRNA_REALM` → `/etc/norrna/realm` → 与 `norrna` 同目录 → `PATH`。

---

## 编译（可选）

需要 Rust 1.80+：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo build --release
```

产物：`target/release/norrna-manager`、`target/release/norrna`。

本地脚本安装（不经过 GitHub）：

```bash
bash scripts/norrna_manager.sh webport=3000 agentport=3001
bash scripts/norrna_agent.sh server=127.0.0.1:3001 apikey=你的KEY
```

不装 systemd 时：

```bash
./norrna-manager --webport 3000 --agentport 3001 --data-dir ./data
./norrna api --server 127.0.0.1:3001 --key YOUR_KEY
```

---

## 常见问题

**面板里 Agent 不是绿色**

1. 节点：`journalctl -u norrna-agent -n 50 --no-pager`
2. 面板：`journalctl -u norrna-manager -n 50 --no-pager`
3. 两边 `--version` 必须一致
4. 同机请用 `server=127.0.0.1:3001`；跨机放行 3001

日志 `TCP connected` 后马上 `Decryption failed` / `unexpected end of file`：面板和 Agent 版本不一致。按上面「更新」把两边都换成 `releases/latest`。

**一键安装下到旧 Agent**

确认面板旁有最新文件：

```bash
ls -lh /etc/norrna-manager/norrna-manager /etc/norrna-manager/norrna
/etc/norrna-manager/norrna-manager --version
/etc/norrna-manager/norrna --version
```

升级面板时务必连 `norrna` 一起换，或走 `--update`。

---

## 仓库结构

```
crates/norrna-manager/  面板
crates/norrna/          Agent（调度 Realm / 端口复用）
crates/norrna-proto/    控制协议
scripts/                一键安装与 systemd
```
