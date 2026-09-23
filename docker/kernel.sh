#!/bin/sh
# kernel.sh —— 容器化内核开发环境薄壳（Firecracker devtool 精神：工具链
# 钉死在镜像里，宿主只 docker run）。
#
# 用法：
#   kernel.sh clone <git-url> [--ref <ref>]   源码克隆进 named volume
#                                             （ext4 大小写敏感，规避 APFS 坑）
#   kernel.sh defconfig [name]                make <name>（缺省 defconfig）
#   kernel.sh menuconfig                      改配置（.config 在 volume 内）
#   kernel.sh build [-j N]                    make Image modules + compile_commands.json
#   kernel.sh cc                              只重新生成 compile_commands.json
#   kernel.sh export [dest]                   最小树导出到宿主（KERNEL_PATH 指它）
#   kernel.sh shell                           容器内交互 bash（neovim 党用容器内 clangd）
#
# 环境变量：
#   KERNEL_VOLUME   volume 名（缺省 virtuoso-kernel）
#   KERNEL_ARCH     arm64 | x86_64 | riscv64（缺省 arm64）
#   KERNEL_IMAGE    镜像（缺省 ghcr.io/yxalix/virtuoso-kernel:latest；
#                   拉取失败自动回落本地构建 docker/Dockerfile.kernel）
#   KERNEL_REF      clone 缺省 ref（缺省 master）
#
# 测试主循环在宿主原生跑：export 出的树 + `virtuoso doctor / build / test`。

set -eu

VOLUME="${KERNEL_VOLUME:-virtuoso-kernel}"
ARCH="${KERNEL_ARCH:-arm64}"
REF="${KERNEL_REF:-master}"
IMAGE="${KERNEL_IMAGE:-ghcr.io/yxalix/virtuoso-kernel:latest}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# arch → 容器内 make 环境前缀（arm64 在 arm64 容器 = 原生前端，其余交叉）
make_env() {
    case "$ARCH" in
        arm64)   echo "ARCH=arm64" ;;
        x86_64)  echo "ARCH=x86_64 CROSS_COMPILE=x86_64-linux-gnu-" ;;
        riscv64) echo "ARCH=riscv64 CROSS_COMPILE=riscv64-linux-gnu-" ;;
        *) echo "kernel.sh: 未知 KERNEL_ARCH=$ARCH（arm64|x86_64|riscv64）" >&2; exit 2 ;;
    esac
}

# arch → clangd --target triple（.clangd 的 --target 行替换值）
clangd_target() {
    case "$ARCH" in
        arm64)   echo "aarch64-linux-gnu" ;;
        x86_64)  echo "x86_64-linux-gnu" ;;
        riscv64) echo "riscv64-linux-gnu" ;;
    esac
}

# 内核镜像相对路径（verify 消费的最小树形态）
kernel_image_rel() {
    case "$ARCH" in
        arm64)   echo "arch/arm64/boot/Image" ;;
        x86_64)  echo "arch/x86/boot/bzImage" ;;
        riscv64) echo "arch/riscv/boot/Image" ;;
    esac
}

docker_cmd() {
    # 通用容器调用：volume 挂 /ksrc（workspace），导出目录挂 /out
    docker run --rm -i ${DOCKER_TTY:-} \
        -v "$VOLUME:/ksrc" \
        -v "$SCRIPT_DIR/../target/kernel:/out" \
        -w /ksrc \
        "$IMAGE" "$@"
}

ensure_image() {
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        echo "kernel.sh: pulling $IMAGE ..." >&2
        if ! docker pull "$IMAGE" 2>/dev/null; then
            echo "kernel.sh: 拉取失败，回落本地构建 $SCRIPT_DIR/Dockerfile.kernel" >&2
            docker build -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.kernel" "$SCRIPT_DIR"
        fi
    fi
}

cmd="${1:-help}"; shift || true

case "$cmd" in
clone)
    [ $# -ge 1 ] || { echo "usage: kernel.sh clone <git-url> [--ref <ref>]" >&2; exit 2; }
    url="$1"; shift || true
    while [ $# -gt 0 ]; do
        case "$1" in
            --ref) REF="$2"; shift 2 ;;
            *) echo "kernel.sh: 未知参数 $1" >&2; exit 2 ;;
        esac
    done
    ensure_image
    docker run --rm -i -v "$VOLUME:/ksrc" "$IMAGE" \
        sh -c "git clone --depth 1 --branch '$REF' '$url' /tmp/k && \
               cp -a /tmp/k/. /ksrc/"
    # clangd 配置按目标架构落进源码根（compile_commands.json 的消费侧）
    docker run --rm -i -v "$VOLUME:/ksrc" -v "$SCRIPT_DIR:/cfg:ro" "$IMAGE" \
        sh -c "sed 's/--target=[a-z0-9_-]*/--target=$(clangd_target)/' /cfg/.clangd > /ksrc/.clangd"
    echo "kernel.sh: cloned $url@$REF → volume $VOLUME（.clangd 已按 $ARCH 配好）"
    ;;
defconfig)
    ensure_image
    docker_cmd sh -c "$(make_env) make ${1:-defconfig}"
    ;;
menuconfig)
    ensure_image
    DOCKER_TTY="-t" docker_cmd sh -c "$(make_env) make menuconfig"
    ;;
build)
    JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 8)"
    while [ $# -gt 0 ]; do
        case "$1" in
            -j) JOBS="$2"; shift 2 ;;
            *) echo "kernel.sh: 未知参数 $1" >&2; exit 2 ;;
        esac
    done
    ensure_image
    docker_cmd sh -c "$(make_env) make -j$JOBS Image modules && \
        python3 scripts/compile_commands.py"
    echo "kernel.sh: built Image + modules；clangd 索引数据（compile_commands.json）已生成"
    ;;
cc)
    ensure_image
    docker_cmd python3 scripts/compile_commands.py
    ;;
export)
    dest="/out/${1:-$ARCH}"
    img="$(kernel_image_rel)"
    ensure_image
    docker_cmd sh -c "$(make_env) make INSTALL_MOD_PATH=/tmp/mods modules_install >/dev/null && \
        rm -rf '$dest' && mkdir -p '$dest/$(dirname "$img")' '$dest' && \
        cp Makefile .config '$dest/' && \
        cp '$img' '$dest/$img' && \
        cp -a /tmp/mods/lib '$dest/lib'"
    echo "kernel.sh: exported minimal tree → target/kernel/$ARCH"
    echo "  KERNEL_PATH 指到该目录（virtuoso.toml kernel_path）即可跑 virtuoso 主循环"
    ;;
shell)
    ensure_image
    DOCKER_TTY="-t" docker_cmd /bin/bash
    ;;
help|*)
    sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
