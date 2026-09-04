#!/bin/bash
set -e
ROOT="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-/opt/zelay}"
WEBPORT="${WEBPORT:-3000}"
AGENTPORT="${AGENTPORT:-3001}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "未检测到 cargo，正在安装 rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "[+] 编译 release"
cd "$ROOT"
cargo build --release

echo "[+] 安装到 $PREFIX"
mkdir -p "$PREFIX/data"
cp -f target/release/zelay-manager target/release/zelay "$PREFIX/"
chmod +x "$PREFIX/zelay-manager" "$PREFIX/zelay"

echo
echo "编译完成。"
echo "启动面板："
echo "  $PREFIX/zelay-manager --webport $WEBPORT --agentport $AGENTPORT --data-dir $PREFIX/data"
echo
echo "启动 Agent（先在面板创建 Agent 拿到 key）："
echo "  $PREFIX/zelay api --server 127.0.0.1:$AGENTPORT --key YOUR_KEY"
