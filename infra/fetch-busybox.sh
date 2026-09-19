#!/bin/bash
# Ensure a per-arch static BusyBox binary for initrd assembly.
#
# 供给链（按序尝试，命中即返回）：
#   0. 本地缓存            infra/busybox/bin/busybox-<arch>
#   1. BUSYBOX_DL_URL      显式完整资产 URL（wget）
#   2. gh release download 自动携带认证 —— 私有仓库 release 也可下载
#   3. 直链 wget           GitHub Release 公开资产（CI / 无 gh 环境）
#   4. 源码编译兜底         archive 归档源：busybox.net -> GitHub mirror
#
# Release 由 .github/workflows/busybox-release.yml 生成（gh CLI 发布）：
#   tag:   busybox-v<version>
#   asset: busybox-<version>-linux-{arm64,x86_64,riscv64}
#
# 相关 .env 变量（均可选，见 .env.example）：
#   BUSYBOX_VERSION      版本（默认 1.36.1）
#   BUSYBOX_RELEASE_REPO 发布 busybox-v* release 的 GitHub 仓库（owner/repo）
#   BUSYBOX_DL_URL       直接指定完整资产 URL（优先级最高）
#   BUSYBOX_SOURCE_BUILD 设为 1 强制源码编译（离线/内网环境）

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CACHE_DIR="${SCRIPT_DIR}/busybox/bin"
BUSYBOX_SRC_DIR="${SCRIPT_DIR}/busybox/busybox"

if [ -f "${PROJECT_ROOT}/.env" ]; then
    set -a
    . "${PROJECT_ROOT}/.env"
    set +a
fi
BUSYBOX_VERSION="${BUSYBOX_VERSION:-1.36.1}"

HOST_ARCH="$(uname -m)"
ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
    aarch64|arm64)  ARCH="arm64" ;;
    x86_64|amd64)   ARCH="x86_64" ;;
    riscv64)        ARCH="riscv64" ;;
    *) echo "ERROR: Unsupported ARCH=$ARCH"; exit 1 ;;
esac

BIN="${CACHE_DIR}/busybox-${ARCH}"
ASSET="busybox-${BUSYBOX_VERSION}-linux-${ARCH}"
TAG="busybox-v${BUSYBOX_VERSION}"

# wget 下载 + ELF 魔数校验 + 落盘缓存；成功返回 0
try_wget_download() {
    local url="$1"
    echo "Fetching ${ASSET} from ${url}"
    wget -q -O "${BIN}.tmp" "$url" || { rm -f "${BIN}.tmp"; return 1; }
    if ! head -c 4 "${BIN}.tmp" | grep -q $'\x7fELF'; then
        rm -f "${BIN}.tmp"; return 1
    fi
    chmod +x "${BIN}.tmp"
    mv "${BIN}.tmp" "$BIN"
    echo "BusyBox: downloaded (${ARCH})"
}

# 推导发布仓库：显式 BUSYBOX_RELEASE_REPO -> GitHub remote 自动推导
resolve_repo() {
    if [ -n "${BUSYBOX_RELEASE_REPO:-}" ]; then
        printf '%s' "$BUSYBOX_RELEASE_REPO"
        return 0
    fi
    local origin
    origin="$(git -C "$PROJECT_ROOT" remote get-url origin 2>/dev/null || true)"
    case "$origin" in
        *github.com[:/]*)
            printf '%s' "$origin" | sed -E 's#.*github\.com[:/]##; s#\.git$##'
            ;;
    esac
}

if [ -x "$BIN" ]; then
    echo "BusyBox: cached (${ARCH})"
    exit 0
fi

mkdir -p "$CACHE_DIR"

# ---------- 1/2/3) Release 下载 ----------
if [ "${BUSYBOX_SOURCE_BUILD:-0}" != "1" ]; then
    # 1) 显式 URL 最高优先级
    if [ -n "${BUSYBOX_DL_URL:-}" ] && try_wget_download "$BUSYBOX_DL_URL"; then
        exit 0
    fi

    repo="$(resolve_repo)"
    if [ -n "$repo" ]; then
        # 2) gh release download：自带认证，私有仓库可用
        if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
            echo "Fetching ${ASSET} via gh from ${repo} (${TAG})"
            if gh release download "$TAG" -R "$repo" -p "$ASSET" -O "${BIN}.tmp" 2>/dev/null \
               && head -c 4 "${BIN}.tmp" | grep -q $'\x7fELF'; then
                chmod +x "${BIN}.tmp"
                mv "${BIN}.tmp" "$BIN"
                echo "BusyBox: downloaded via gh (${ARCH})"
                exit 0
            fi
            rm -f "${BIN}.tmp"
            echo "WARNING: gh release download failed, trying plain wget"
        fi
        # 3) 直链 wget（公开仓库 / CI 无 gh）
        if try_wget_download "https://github.com/${repo}/releases/download/${TAG}/${ASSET}"; then
            exit 0
        fi
        echo "WARNING: release download failed, falling back to source build"
    else
        echo "WARNING: no release source available."
        echo "  Set BUSYBOX_RELEASE_REPO=<owner>/<repo> in .env (or push to GitHub with the busybox-release workflow)."
    fi
fi

# ---------- 4) 源码编译兜底（宿主架构；交叉时明确告警） ----------
echo "Building BusyBox ${BUSYBOX_VERSION} from source (host arch: ${HOST_ARCH})..."
mkdir -p "${SCRIPT_DIR}/busybox"

try_archive() {   # <url> <tar-flags> <解包后目录名>
    local url="$1" flags="$2" sub="$3"
    local tmp="${SCRIPT_DIR}/busybox/src.tar"
    echo "  source archive: ${url}"
    wget -q -O "$tmp" "$url" || { rm -f "$tmp"; return 1; }
    tar ${flags} "$tmp" -C "${SCRIPT_DIR}/busybox" || { rm -f "$tmp"; return 1; }
    rm -f "$tmp"
    mv "${SCRIPT_DIR}/busybox/${sub}" "${BUSYBOX_SRC_DIR}"
}

if [ ! -d "${BUSYBOX_SRC_DIR}" ]; then
    tag="${BUSYBOX_VERSION//./_}"   # busybox tag 命名: 1.36.1 -> 1_36_1
    try_archive "https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2" "-xjf" "busybox-${BUSYBOX_VERSION}" \
        || try_archive "https://github.com/mirror/busybox/archive/refs/tags/${tag}.tar.gz" "-xzf" "busybox-${tag}" \
        || { echo "ERROR: all BusyBox source archives failed"; exit 1; }
fi

cd "${BUSYBOX_SRC_DIR}"
make defconfig
sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
make -j"$(nproc)"
cp busybox "$BIN"

host_norm="$HOST_ARCH"
case "$host_norm" in
    aarch64) host_norm="arm64" ;;
    amd64)   host_norm="x86_64" ;;
esac
if [ "$ARCH" != "$host_norm" ]; then
    echo "WARNING: source-built BusyBox is ${host_norm} but target ARCH=${ARCH}."
    echo "  Cross-arch initramfs needs a prebuilt release binary:"
    echo "  run the busybox-release workflow, then set BUSYBOX_RELEASE_REPO in .env."
fi
echo "BusyBox: built from source (${host_norm} binary cached as busybox-${ARCH})"
