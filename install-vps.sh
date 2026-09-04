#!/bin/bash
set -e
ROOT="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-/opt/norrna}"
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
cp -f target/release/norrna-manager target/release/norrna "$PREFIX/"
chmod +x "$PREFIX/norrna-manager" "$PREFIX/norrna"

echo
echo "编译完成。"
echo "启动面板："
echo "  $PREFIX/norrna-manager --webport $WEBPORT --agentport $AGENTPORT --data-dir $PREFIX/data"
echo
echo "启动 Agent（先在面板创建 Agent 拿到 key）："
echo "  $PREFIX/norrna api --server 127.0.0.1:$AGENTPORT --key YOUR_KEY"
echo
echo "普通转发需要官方 realm，放到 $PREFIX/realm 或 PATH："
echo "  https://github.com/zhboner/realm/releases/tag/v2.9.6"
