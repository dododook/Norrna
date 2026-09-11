#!/bin/bash

# Norrna Manager 一键部署脚本
# 支持安装、更新、卸载

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 默认配置
DEFAULT_WEB_PORT=3000
DEFAULT_AGENT_PORT=3001
INSTALL_DIR="/etc/norrna-manager"
BINARY_NAME="norrna-manager"
SERVICE_NAME="norrna-manager"
REPO_URL="https://github.com/dododook/Norrna"
RELEASE_BASE="https://github.com/dododook/Norrna/releases/latest/download"
DOWNLOAD_URL=""
ARCH_TAG=""
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 解析参数
WEB_PORT=$DEFAULT_WEB_PORT
AGENT_PORT=$DEFAULT_AGENT_PORT
ACTION="install"
DATA_DIR=""

for arg in "$@"; do
    case $arg in
        webport=*|web-port=*)
            WEB_PORT="${arg#*=}"
            ;;
        agentport=*|agent-port=*)
            AGENT_PORT="${arg#*=}"
            ;;
        datadir=*|data-dir=*)
            DATA_DIR="${arg#*=}"
            ;;
        --uninstall|uninstall)
            ACTION="uninstall"
            ;;
        --update|update)
            ACTION="update"
            ;;
        --help|-h|help)
            ACTION="help"
            ;;
        *)
            echo -e "${RED}未知参数: $arg${NC}"
            ACTION="help"
            ;;
    esac
done

# 设置默认数据目录
if [[ -z "$DATA_DIR" ]]; then
    DATA_DIR="${INSTALL_DIR}/data"
fi

# 打印日志函数
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 显示帮助信息
show_help() {
    cat << EOF
${GREEN}Norrna Manager 部署脚本${NC}

${BLUE}用法:${NC}
    bash norrna_manager.sh [选项]

${BLUE}选项:${NC}
    webport=PORT         设置 Web 管理面板端口 (默认: 3000)
    agentport=PORT       设置 Agent 连接端口 (默认: 3001)
    datadir=PATH         设置数据目录 (默认: /etc/norrna-manager/data)
    --update             更新 Norrna Manager
    --uninstall          卸载 Norrna Manager
    --help, -h           显示帮助信息

${BLUE}示例:${NC}
    # 默认安装
    bash norrna_manager.sh

    # 自定义端口安装
    bash norrna_manager.sh webport=8080 agentport=9000

    # 自定义数据目录
    bash norrna_manager.sh webport=3000 agentport=3001 datadir=/data/norrna

    # 更新
    bash norrna_manager.sh --update

    # 卸载
    bash norrna_manager.sh --uninstall

${BLUE}更多信息:${NC}
    项目地址: https://github.com/dododook/Norrna
EOF
}

# 检查是否为 root 用户
check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "请使用 root 权限运行此脚本"
        exit 1
    fi
}

# 检查系统架构
check_architecture() {
    ARCH=$(uname -m)
    case $ARCH in
        x86_64|amd64)
            ARCH_TAG="linux-amd64"
            log_info "检测到系统架构: x86_64"
            ;;
        aarch64|arm64)
            ARCH_TAG="linux-arm64"
            log_info "检测到系统架构: ARM64 (aarch64)"
            ;;
        *)
            log_error "不支持的系统架构: $ARCH（需要 x86_64 或 aarch64）"
            exit 1
            ;;
    esac
    DOWNLOAD_URL="${RELEASE_BASE}/norrna-manager-${ARCH_TAG}"
}

# 检查操作系统
check_os() {
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        OS=$ID
        log_info "检测到操作系统: $PRETTY_NAME"
    else
        log_error "无法识别操作系统"
        exit 1
    fi
}

# 检查端口是否被占用
check_port() {
    local port=$1
    if ss -tuln | grep -q ":${port} "; then
        log_warning "端口 $port 已被占用"
        read -p "是否继续? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            log_info "安装已取消"
            exit 0
        fi
    fi
}

# 停止服务
stop_service() {
    if systemctl is-active --quiet $SERVICE_NAME; then
        log_info "停止 $SERVICE_NAME 服务..."
        systemctl stop $SERVICE_NAME
        log_success "服务已停止"
    fi
}

# 创建安装目录
create_directory() {
    log_info "创建安装目录: $INSTALL_DIR"
    mkdir -p $INSTALL_DIR
    log_success "目录创建成功"
}

find_local_bin() {
    local name="$1"
    local cand
    for cand in \
        "${SCRIPT_DIR}/${name}" \
        "${SCRIPT_DIR}/../target/release/${name}" \
        "${PWD}/${name}" \
        "${PWD}/target/release/${name}"; do
        if [[ -f "$cand" ]]; then
            echo "$cand"
            return 0
        fi
    done
    return 1
}

# 优先从 GitHub Release 下载；没有 Release 或失败则用本地编译产物
download_binary() {
    local dest="${INSTALL_DIR}/${BINARY_NAME}"
    log_info "安装 Norrna Manager..."

    local ok=0
    try_dl() {
        local url="$1"
        [[ -z "$url" ]] && return 1
        log_info "尝试下载: $url"
        if command -v curl &> /dev/null && curl -fsSL -o "$dest" "$url" && [[ -s "$dest" ]]; then
            return 0
        fi
        if command -v wget &> /dev/null && wget -q -O "$dest" "$url" && [[ -s "$dest" ]]; then
            return 0
        fi
        rm -f "$dest"
        return 1
    }
    if try_dl "$DOWNLOAD_URL"; then
        ok=1
    elif [[ "$ARCH_TAG" == "linux-amd64" ]] && try_dl "${RELEASE_BASE}/norrna-manager"; then
        ok=1
    else
        log_warning "GitHub 下载失败（可能还没有对应架构的 Release），改为使用本地文件"
        rm -f "$dest"
    fi
    if [[ "$ok" -eq 0 ]]; then
        local src
        src="$(find_local_bin "$BINARY_NAME" || true)"
        if [[ -z "$src" ]]; then
            log_error "无法获取 ${BINARY_NAME}"
            log_info "请先 cargo build --release，或在 GitHub 发布 Release 并上传二进制"
            exit 1
        fi
        log_info "使用本地文件: $src"
        cp -f "$src" "$dest"
    fi

    if [[ ! -f "$dest" ]]; then
        log_error "安装失败"
        exit 1
    fi
    chmod +x "$dest"

    local agent_src
    agent_src="$(find_local_bin norrna || true)"
    if [[ -z "$agent_src" ]]; then
        local agent_url="${RELEASE_BASE}/norrna-${ARCH_TAG}"
        if command -v curl &> /dev/null && curl -fsSL -o "${INSTALL_DIR}/norrna" "$agent_url" && [[ -s "${INSTALL_DIR}/norrna" ]]; then
            agent_src="${INSTALL_DIR}/norrna"
        elif [[ "$ARCH_TAG" == "linux-amd64" ]] && command -v curl &> /dev/null && curl -fsSL -o "${INSTALL_DIR}/norrna" "${RELEASE_BASE}/norrna" && [[ -s "${INSTALL_DIR}/norrna" ]]; then
            agent_src="${INSTALL_DIR}/norrna"
        fi
    fi
    if [[ -n "$agent_src" && "$agent_src" != "${INSTALL_DIR}/norrna" ]]; then
        cp -f "$agent_src" "${INSTALL_DIR}/norrna"
    fi
    if [[ -f "${INSTALL_DIR}/norrna" ]]; then
        chmod +x "${INSTALL_DIR}/norrna"
        log_info "已同时安装 Agent 二进制: ${INSTALL_DIR}/norrna"
    fi

    log_success "安装完成"
}

# 设置文件权限
set_permissions() {
    log_info "设置文件权限..."
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    mkdir -p "$DATA_DIR"
    log_success "权限设置完成"
}

# 创建 systemd 服务
create_service() {
    log_info "创建 systemd 服务..."
    cat > "/etc/systemd/system/${SERVICE_NAME}.service" << EOF
[Unit]
Description=Norrna Manager
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/${BINARY_NAME} --webport ${WEB_PORT} --agentport ${AGENT_PORT} --data-dir ${DATA_DIR}
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=${SERVICE_NAME}

# 资源限制
LimitNOFILE=1048576
LimitNPROC=1048576

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    log_success "服务创建成功"
}

# 启动服务
start_service() {
    log_info "启动 $SERVICE_NAME 服务..."
    systemctl enable $SERVICE_NAME
    systemctl start $SERVICE_NAME
    sleep 2
    if systemctl is-active --quiet $SERVICE_NAME; then
        log_success "服务启动成功"
    else
        log_error "服务启动失败，请查看日志: journalctl -u $SERVICE_NAME -n 50"
        exit 1
    fi
}

# 显示安装信息
show_install_info() {
    local SERVER_IP=$(curl -s ifconfig.me || echo "YOUR_SERVER_IP")
    echo
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                  Norrna Manager 安装成功！                      ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo
    echo -e "${BLUE} 安装目录:${NC} $INSTALL_DIR"
    echo -e "${BLUE} 数据目录:${NC} $DATA_DIR"
    echo -e "${BLUE} 访问地址:${NC} http://${SERVER_IP}:${WEB_PORT}"
    echo -e "${BLUE} Agent 端口:${NC} ${AGENT_PORT}"
    echo
    echo -e "${YELLOW} 下一步操作:${NC}"
    echo -e "  1. 访问管理面板: ${BLUE}http://${SERVER_IP}:${WEB_PORT}${NC}"
    echo -e "  2. 创建管理员账号"
    echo -e "  3. 登录并开始使用"
    echo
    echo -e "${YELLOW} 常用命令:${NC}"
    echo -e "  查看状态: ${GREEN}systemctl status $SERVICE_NAME${NC}"
    echo -e "  启动服务: ${GREEN}systemctl start $SERVICE_NAME${NC}"
    echo -e "  停止服务: ${GREEN}systemctl stop $SERVICE_NAME${NC}"
    echo -e "  重启服务: ${GREEN}systemctl restart $SERVICE_NAME${NC}"
    echo -e "  查看日志: ${GREEN}journalctl -u $SERVICE_NAME -f${NC}"
    echo
    echo -e "${YELLOW} 更多信息:${NC}"
    echo -e "  GitHub: ${BLUE}https://github.com/dododook/Norrna${NC}"
    echo
}

# 安装函数
install() {
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║              开始安装 Norrna Manager                            ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo
    check_root
    check_os
    check_architecture
    log_info "配置信息:"
    echo -e "  Web 端口: ${GREEN}${WEB_PORT}${NC}"
    echo -e "  Agent 端口: ${GREEN}${AGENT_PORT}${NC}"
    echo -e "  安装目录: ${GREEN}${INSTALL_DIR}${NC}"
    echo -e "  数据目录: ${GREEN}${DATA_DIR}${NC}"
    echo
    check_port $WEB_PORT
    check_port $AGENT_PORT
    if [[ -f "/etc/systemd/system/${SERVICE_NAME}.service" ]]; then
        log_warning "检测到已安装 Norrna Manager"
        stop_service
    fi
    create_directory
    download_binary
    set_permissions
    create_service
    start_service
    show_install_info
}

# 更新函数
update() {
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║              开始更新 Norrna Manager                            ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo
    check_root
    check_architecture
    if [[ ! -f "/etc/systemd/system/${SERVICE_NAME}.service" ]]; then
        log_error "未检测到已安装的 Norrna Manager"
        log_info "请使用安装命令进行安装"
        exit 1
    fi
    log_info "备份当前版本..."
    if [[ -f "${INSTALL_DIR}/${BINARY_NAME}" ]]; then
        cp "${INSTALL_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}.bak"
        log_success "备份完成"
    fi
    stop_service
    download_binary
    start_service
    echo
    log_success "更新完成！"
    echo
    log_info "如果遇到问题，可以回滚到之前的版本:"
    echo -e "  ${GREEN}mv ${INSTALL_DIR}/${BINARY_NAME}.bak ${INSTALL_DIR}/${BINARY_NAME}${NC}"
    echo -e "  ${GREEN}systemctl restart $SERVICE_NAME${NC}"
    echo
}

# 卸载函数
uninstall() {
    echo -e "${YELLOW}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║              开始卸载 Norrna Manager                            ║${NC}"
    echo -e "${YELLOW}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo
    check_root
    log_warning "此操作将删除 Norrna Manager 及所有数据"
    read -p "确认卸载? (yes/no): " -r
    echo
    if [[ ! $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        log_info "卸载已取消"
        exit 0
    fi
    if [[ -f "/etc/systemd/system/${SERVICE_NAME}.service" ]]; then
        log_info "停止并禁用服务..."
        systemctl stop $SERVICE_NAME 2>/dev/null || true
        systemctl disable $SERVICE_NAME 2>/dev/null || true
        log_success "服务已停止"
    fi
    if [[ -f "/etc/systemd/system/${SERVICE_NAME}.service" ]]; then
        log_info "删除服务文件..."
        rm -f "/etc/systemd/system/${SERVICE_NAME}.service"
        systemctl daemon-reload
        log_success "服务文件已删除"
    fi
    if [[ -d "$INSTALL_DIR" ]]; then
        log_info "删除安装目录..."
        rm -rf "$INSTALL_DIR"
        log_success "安装目录已删除"
    fi
    echo
    log_success "Norrna Manager 已完全卸载！"
    echo
    log_info "感谢使用 Norrna Manager"
    echo
}

# 主函数
main() {
    case $ACTION in
        install)
            install
            ;;
        update)
            update
            ;;
        uninstall)
            uninstall
            ;;
        help)
            show_help
            ;;
        *)
            log_error "未知操作: $ACTION"
            show_help
            exit 1
            ;;
    esac
}

# 执行主函数
main
