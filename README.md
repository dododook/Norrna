# Zelay 还原工程（可部署）

这是从你编译的 `zelay` / `zelay-manager` ELF **逆向还原**出来的可运行项目，不是原仓库源码逐行拷贝。

- 面板 HTML/CSS/JS 是从二进制里完整抽出来的（含中文）
- 管理端 API、JSON 存储、JWT Cookie、Agent 控制通道按二进制字符串还原
- 转发内核是能工作的 TCP/UDP relay + 端口复用，**不是**完整 Realm（没有 kaminari/WS/TLS 伪装、zero-copy、MPTCP、PROXY protocol）
- Agent 与面板之间的包格式是这次还原用的 JSON 帧，**不能**和原版二进制混用

对照源码时：页面和 API 路径应对得上；内核细节会有差。

## 在 Ubuntu/Debian VPS 上编译

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# 上传本目录后
cd zelay-rebuild
cargo build --release
# 产物：
#   target/release/zelay-manager
#   target/release/zelay
```

## 用你原来的一键脚本安装

脚本已放进 `scripts/`，启动命令和原版一致：

- 面板：`/etc/zelay-manager/zelay-manager --webport --agentport --data-dir`
- Agent：`/etc/zelay/zelay api -c /etc/zelay/zelay.conf --server --key`
- 数据：`/etc/zelay-manager/data`、`/etc/zelay/instances`

你给的脚本里 `DOWNLOAD_URL` / `BINARY_URL` 是空的，所以改成：**有 URL 就下载，没有就拷贝本地编译产物**。

```bash
cd zelay-rebuild
cargo build --release

# 面板机（root）
bash scripts/zelay_manager.sh webport=3000 agentport=3001
# 会安装 zelay-manager，若同目录有 zelay 也会拷到 /etc/zelay-manager/zelay
# 供面板地址 /zelay 和 /zelay_agent.sh 使用
```

浏览器打开 `http://IP:3000` 创建管理员、添加 Agent。面板里的部署命令会变成：

```bash
bash <(curl -fsSL http://IP:3000/zelay_agent.sh) \
  server=IP:3001 \
  apikey=你的KEY \
  dns=223.5.5.5:53,119.29.29.29:53
```

在节点机执行这条即可（脚本会从面板下载 `zelay` 二进制）。

同一台机器也可以：

```bash
bash scripts/zelay_agent.sh server=127.0.0.1:3001 apikey=你的KEY
```

面板显示 Agent 在线后，创建转发：监听 `0.0.0.0` + 端口，远程填目标 `IP:端口`，点启动。

IPv6 全接口监听填 `::` 时，内部会把 `:::端口` 解析成 `[::]:端口`。

## systemd（可选）

```bash
# 先改 scripts/zelay-agent.service 里的 --key
cp scripts/zelay-manager.service /etc/systemd/system/
cp scripts/zelay-agent.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now zelay-manager
systemctl enable --now zelay-agent
```

## 端口复用（简化实现）

| multiplex_mode | 含义 | 行为 |
|---|---|---|
| 0 | 普通转发 | listen ↔ remote |
| 1 | 服务端 | 入站带 `ZELAYMX1` 头则转到头里的最终目标，否则走默认 remote |
| 2 | 客户端 | 连到 remote（IX），先发 owner_user_id + final_target 头，再透传 |

UDP 头魔数为 `ZELAY_UDP`。

## 已知与原版的差异

1. 没有 MySQL 握手伪装、WS/TLS 混淆、PROXY protocol、MPTCP
2. 控制通道未做 ChaCha 加密（原版二进制里有 `expand 32-byte k`）
3. JWT 仍用二进制里的默认串 `your-jwt-secret-change-this-in-production-keep-it-very-secret`
4. 密码仍是 MD5，和原版一致
5. 面板部署命令走本机 `/zelay_agent.sh`（你的脚本），不再拉取 GitHub 原版二进制

## 目录

```
zelay-rebuild/
  crates/zelay-manager/   面板
  crates/zelay/           Agent + 转发
  crates/zelay-proto/     控制协议
  scripts/                systemd 单元
```
