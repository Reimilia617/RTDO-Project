#!/usr/bin/env bash
# rtdo — 一键安装 / 卸载脚本（v0.5.0）
#
# 用法：
#   curl 安装：  curl -fsSL https://raw.githubusercontent.com/Reimilia617/RTDO-Project/main/rtdo.sh | sudo sh -s -- --install
#   git clone：  git clone https://github.com/Reimilia617/RTDO-Project.git && cd RTDO-Project && sudo ./rtdo.sh --install
#   预编译安装： sudo ./rtdo.sh --install --prebuilt-dir /path/to/dir   （目录内需有 rtdo 与 rtdo-sudod）
#   卸载：       sudo ./rtdo.sh --uninstall        （或先下载脚本再执行）
#   状态：       sudo ./rtdo.sh --status
#
# 说明：
#   * --install：拉取/使用源码 -> 编译 rtdo（Rust）与 rtdo-sudod（Go）-> 安装到系统
#     （/usr/local/bin/rtdo setuid root、/usr/local/lib/rtdo/rtdo-sudod、
#      /etc/rtdo、/etc/pam.d/rtdo）-> 拦截 sudo（原版移动到 /usr/bin/sudo.real，
#      可用 sudo-force 调用原版）-> 注册并启动 systemd 服务 rtdo-sudod。
#   * --prebuilt-dir <目录>：跳过编译，直接用该目录里现成的 rtdo / rtdo-sudod。
#     供 Reimilia 的 Binary 安装方式与 CI 发布产物使用，安装逻辑仍由本脚本独家实现。
#   * --uninstall：停止并删除服务、删除配置与二进制、还原 sudo 为可用模式。
set -euo pipefail

VERSION="0.5.0"
REPO="Reimilia617/RTDO-Project"
RAW_URL="https://raw.githubusercontent.com/$REPO/main/rtdo.sh"
TARBALL_URL="https://github.com/$REPO/archive/refs/heads/main.tar.gz"

RTDO_BIN="/usr/local/bin/rtdo"
DAEMON_DIR="/usr/local/lib/rtdo"
DAEMON_BIN="$DAEMON_DIR/rtdo-sudod"
CONF_DIR="/etc/rtdo"
CONF="$CONF_DIR/rtdo.conf"
PAM_FILE="/etc/pam.d/rtdo"
UNIT="/etc/systemd/system/rtdo-sudod.service"
SUDO="/usr/bin/sudo"
SUDO_REAL="/usr/bin/sudo.real"
SUDO_FORCE="/usr/bin/sudo-force"
MARKER="$CONF_DIR/sudo-disabled"

# 由 --prebuilt-dir 设置：非空则跳过编译，直接使用该目录中的二进制
PREBUILT_DIR="${RTDO_PREBUILT_DIR:-}"

say()  { printf '%s\n' "$*"; }
die()  { say "错误 / error: $*" >&2; exit 1; }

usage() {
  cat <<EOF
rtdo v$VERSION — Root Task Do：交互式、环境自适应的提权工具（替代 sudo）
rtdo v$VERSION — Root Task Do: interactive privilege elevation tool (sudo alternative)

用法 / usage:
  curl 安装 / curl install:
    curl -fsSL $RAW_URL | sudo sh -s -- --install
  git clone 安装 / git-clone install:
    git clone https://github.com/$REPO.git && cd RTDO-Project && sudo ./rtdo.sh --install
  预编译安装 / prebuilt install:
    sudo ./rtdo.sh --install --prebuilt-dir /path/to/dir
  卸载 / uninstall:
    sudo ./rtdo.sh --uninstall
  状态 / status:
    sudo ./rtdo.sh --status

参数 / options:
  --install      安装（编译或使用预编译二进制、注册 systemd 服务、拦截 sudo）/ install (build, install, register service, intercept sudo)
  --prebuilt-dir <目录>  使用该目录下的 rtdo / rtdo-sudod，跳过编译 / use prebuilt binaries from <dir>, skip building
  --uninstall    卸载（删除配置与二进制、还原 sudo）/ uninstall (remove files, restore sudo)
  --status       查看安装状态 / show install status
  --help, -h     帮助 / help
EOF
}

check_root() {
  [ "$(id -u)" -eq 0 ] || die "请以 root 运行（sudo 可能已被 rtdo 拦截，请用 su - 或 sudo-force 提升）\
/ please run as root (sudo may be intercepted by rtdo; use su - or sudo-force)."
}

# 确定源码目录：优先本地 git clone（脚本所在目录或 cwd），否则从 GitHub 下载压缩包。
detect_source() {
  if [ -n "${RTDO_SRC:-}" ] && [ -d "$RTDO_SRC" ]; then
    echo "$RTDO_SRC"; return
  fi
  local here
  here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
  if [ -f "$here/Cargo.toml" ] && [ -d "$here/daemon" ]; then
    echo "$here"; return
  fi
  local tmp; tmp="$(mktemp -d)"
  say "==> 下载源码 / downloading source: $TARBALL_URL"
  curl -fsSL --retry 3 --retry-delay 2 "$TARBALL_URL" -o "$tmp/src.tar.gz" \
    || die "下载源码失败 / failed to download source（可先 git clone 后本地安装）"
  tar -xzf "$tmp/src.tar.gz" -C "$tmp"
  echo "$tmp/RTDO-Project-main"
}

check_os_prereqs() {
  command -v systemctl >/dev/null 2>&1 || die "未找到 systemd（systemctl）/ systemd not found."
  command -v sudo >/dev/null 2>&1 || die "未安装 sudo，请先安装并配置 sudo（apt install sudo; usermod -aG sudo <用户>）/ sudo not found; install and configure sudo first."
}

check_build_prereqs() {
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo（Rust 工具链）。请先安装: https://rustup.rs / cargo not found; install Rust first."
  command -v go >/dev/null 2>&1 || die "未找到 go（Go 工具链，用于编译守护进程）。请先安装: https://go.dev/dl / go not found; install Go first."
  if [ ! -f /usr/include/security/pam_appl.h ]; then
    say "==> 缺少 libpam 开发头文件，尝试安装… / installing libpam dev headers…"
    if command -v apt-get >/dev/null 2>&1; then apt-get update -qq && apt-get install -y libpam0g-dev
    elif command -v dnf >/dev/null 2>&1; then dnf install -y pam-devel
    elif command -v pacman >/dev/null 2>&1; then pacman -S --noconfirm pam
    else die "请手动安装 libpam 开发头文件 / install libpam dev headers manually."; fi
  fi
}

# 源码安装所需的全部前置条件（保留旧函数名，兼容既有调用）
check_prereqs() { check_os_prereqs; check_build_prereqs; }

build() {
  local src="$1"
  say "==> 编译 rtdo（Rust）/ building rtdo (Rust)…"
  ( cd "$src" && cargo build --release )
  say "==> 编译 rtdo-sudod（Go）/ building rtdo-sudod (Go)…"
  ( cd "$src/daemon" && CGO_ENABLED=0 go build -trimpath -buildvcs=false -ldflags "-s -w" -o rtdo-sudod . )
  [ -x "$src/target/release/rtdo" ] || die "rtdo 编译失败 / rtdo build failed."
  [ -x "$src/daemon/rtdo-sudod" ] || die "rtdo-sudod 编译失败 / rtdo-sudod build failed."
}

write_unit() {
  cat > "$UNIT" <<EOF
[Unit]
Description=rtdo sudo interception daemon (rtdo-sudod)
Documentation=https://github.com/$REPO
After=local-fs.target

[Service]
Type=simple
ExecStart=$DAEMON_BIN serve
RuntimeDirectory=rtdo
RuntimeDirectoryMode=0755
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF
}

cmd_install() {
  check_root
  check_os_prereqs

  local rtdo_src daemon_src
  if [ -n "$PREBUILT_DIR" ]; then
    [ -d "$PREBUILT_DIR" ] || die "预编译目录不存在 / prebuilt dir not found: $PREBUILT_DIR"
    rtdo_src="$PREBUILT_DIR/rtdo"
    daemon_src="$PREBUILT_DIR/rtdo-sudod"
    [ -f "$rtdo_src" ]   || die "预编译目录缺少 rtdo / missing rtdo in $PREBUILT_DIR"
    [ -f "$daemon_src" ] || die "预编译目录缺少 rtdo-sudod / missing rtdo-sudod in $PREBUILT_DIR"
    say "==> 使用预编译二进制，跳过编译 / using prebuilt binaries, skipping build: $PREBUILT_DIR"
    chmod 0755 "$rtdo_src" "$daemon_src" 2>/dev/null || true
  else
    local src; src="$(detect_source)"
    check_build_prereqs
    build "$src"
    rtdo_src="$src/target/release/rtdo"
    daemon_src="$src/daemon/rtdo-sudod"
  fi

  say "==> 安装文件 / installing files…"
  install -D -m 4755 -o root -g root "$rtdo_src" "$RTDO_BIN"
  install -D -m 0755 -o root -g root "$daemon_src" "$DAEMON_BIN"

  # 生成默认配置与 PAM 服务文件（rtdo --init 需要 root，已满足）
  "$RTDO_BIN" --init || true

  say "==> 拦截 sudo（原版移动为 $SUDO_REAL，可用 sudo-force 调用）/ intercepting sudo…"
  if [ -e "$SUDO" ] && [ ! -e "$SUDO_REAL" ]; then
    mv -f "$SUDO" "$SUDO_REAL"
  fi
  install -m 0755 "$DAEMON_BIN" "$SUDO"
  install -m 0755 "$DAEMON_BIN" "$SUDO_FORCE"
  touch "$MARKER"
  # 清理旧版全局别名文件（0.1.0 方案，已废弃）
  rm -f /etc/profile.d/rtdo-alias.sh

  say "==> 注册并启动 systemd 服务 / registering & starting systemd service…"
  write_unit
  systemctl daemon-reload
  systemctl enable --now rtdo-sudod.service

  cat <<EOF

rtdo v$VERSION 安装完成 / installed.
  用法 / usage:            rtdo <命令>
  首次运行自动设置 root 密码（等价于 sudo passwd）/ first run auto-sets the root password.
  sudo 已被拦截 / sudo is intercepted:
     使用 rtdo 代替 / use rtdo instead;  原版 sudo 请用 / original sudo: sudo-force <命令>
  systemd 服务 / service:  rtdo-sudod.service（sudo systemctl status rtdo-sudod）
  卸载 / uninstall:        sudo $0 --uninstall
EOF
}

cmd_uninstall() {
  check_root
  say "==> 停止并移除服务 / stopping & removing service…"
  systemctl disable --now rtdo-sudod.service 2>/dev/null || true
  rm -f "$UNIT"
  systemctl daemon-reload

  say "==> 还原 sudo / restoring sudo…"
  rm -f "$SUDO" "$SUDO_FORCE"
  if [ -e "$SUDO_REAL" ]; then
    mv -f "$SUDO_REAL" "$SUDO"
    say "    sudo 已还原 / sudo restored."
  else
    say "    未找到 $SUDO_REAL（原版 sudo 未被移动过）/ original sudo backup not found."
  fi

  say "==> 删除 rtdo 文件 / removing rtdo files…"
  rm -f "$RTDO_BIN"
  rm -rf "$DAEMON_DIR"
  rm -f "$PAM_FILE"
  rm -rf "$CONF_DIR"

  cat <<EOF

rtdo 已卸载 / uninstalled.
  sudo 已还原为可用模式 / sudo is back to normal.
  如需重新安装 / to reinstall: curl -fsSL $RAW_URL | sudo sh -s -- --install
EOF
}

cmd_status() {
  check_root
  say "rtdo v$VERSION 状态 / status:"
  if [ -x "$RTDO_BIN" ]; then
    say "  rtdo 二进制:    已安装 / installed ($RTDO_BIN)"
    say "  版本 / version:  $("$RTDO_BIN" --version 2>/dev/null || echo '?')"
  else
    say "  rtdo 二进制:    未安装 / not installed"
  fi
  if [ -e "$MARKER" ]; then
    say "  sudo 拦截:       已启用 / enabled (原版: $SUDO_REAL)"
  else
    say "  sudo 拦截:       未启用 / disabled"
  fi
  if systemctl is-active --quiet rtdo-sudod.service 2>/dev/null; then
    say "  systemd 服务:    运行中 / running (rtdo-sudod)"
  else
    say "  systemd 服务:    未运行 / not running"
  fi
  if [ -f "$CONF" ]; then
    say "  配置 / config:    $CONF"
  fi
}

main() {
  local action=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --install|-i)     action=install ;;
      --uninstall|-u)   action=uninstall ;;
      --status|-s)      action=status ;;
      --prebuilt-dir)   shift; [ "$#" -gt 0 ] || die "--prebuilt-dir 需要一个目录参数 / --prebuilt-dir needs a directory"; PREBUILT_DIR="$1" ;;
      --prebuilt-dir=*) PREBUILT_DIR="${1#*=}" ;;
      --help|-h)        action=help ;;
      "")               ;;
      *) say "未知参数 / unknown option: $1" >&2; usage; exit 2 ;;
    esac
    shift
  done

  case "$action" in
    install)   cmd_install ;;
    uninstall) cmd_uninstall ;;
    status)    cmd_status ;;
    *)         usage ;;
  esac
}

main "$@"
