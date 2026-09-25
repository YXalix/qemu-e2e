#!/bin/sh
# kernel.sh —— 容器化内核开发环境薄壳（Firecracker devtool 精神：工具链
# 钉死在镜像里，宿主只 docker run）。
#
# 源码权威存放在 named volume（容器侧 ext4：大小写敏感 + 构建性能），宿主
# 经平台视图直接读写：macOS = OrbStack 视图（~/OrbStack/docker/volumes/<卷>），
# Linux = volume 本体（/var/lib/docker/volumes/<卷>/_data，rootless 在 $HOME 下）。
# `kernel.sh path` 打印宿主可见路径——AI / 编辑器 / git 以它为工作目录，
# 构建走容器 make，验证走宿主 virtuoso：单 AI 单工作区闭环。
#
# 用法：
#   kernel.sh clone <git-url> [--ref <ref>]   源码克隆进 named volume
#   kernel.sh defconfig [name]                make <name>（缺省 defconfig）
#   kernel.sh menuconfig                      改配置（.config 在 volume 内）
#   kernel.sh build [-j N]                    make Image modules + compile_commands.json
#                                             （CDB 改写为宿主路径形态）
#   kernel.sh cc                              只重新生成 CDB（宿主路径形态）
#   kernel.sh ccr                             只重新生成 CDB（容器 /ksrc 原始形态，devcontainer 用）
#   kernel.sh ccfix                           现有 CDB 原地改写为宿主路径形态
#   kernel.sh path                            打印 volume 的宿主可见路径（AI/编辑器 cwd）
#   kernel.sh export [dest]                   最小树导出到宿主（无宿主视图引擎如
#                                             Docker Desktop 的回退路径）
#   kernel.sh shell                           容器内交互 bash
#
# 多内核切换 = 换卷名（每卷自含源码 + .config + 增量产物，切回免重编）：
#   KERNEL_VOLUME=ksrc-openEuler-6.6 kernel.sh clone <url> --ref OLK-6.6-dev
#   KERNEL_VOLUME=ksrc-mainline      kernel.sh clone <url> --ref master
#
# 环境变量：
#   KERNEL_VOLUME   volume 名（缺省 virtuoso-kernel）
#   KERNEL_ARCH     arm64 | x86_64 | riscv64（缺省 arm64）
#   KERNEL_IMAGE    镜像（缺省 ghcr.io/yxalix/virtuoso-kernel:latest；
#                   拉取失败自动回落本地构建 devkit/docker/Dockerfile.kernel）
#   KERNEL_REF      clone 缺省 ref（缺省 master）
#
# 测试主循环在宿主原生跑：KERNEL_PATH 指 `kernel.sh path` 的输出（或 export
# 出的最小树）+ `virtuoso doctor / build / test`。

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
        *) echo "kernel.sh: 未知 KERNEL_ARCH=${ARCH}（arm64|x86_64|riscv64）" >&2; exit 2 ;;
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

# volume 的宿主可见路径。macOS 走 OrbStack 视图（Docker Desktop 的 volume 在
# VM 虚拟盘里，宿主不可见）；Linux 上 docker volume 本体就在宿主文件系统，
# 取引擎权威 Mountpoint（rootless 时落在 $HOME 下，免 root）。两个平台该路径
# 都是纯宿主路径——build/cc 产出的 compile_commands.json 据此改写，宿主
# clangd 直接消费。
host_path() {
    case "$(uname -s)" in
        Darwin)
            echo "$HOME/OrbStack/docker/volumes/$VOLUME" ;;
        Linux)
            docker volume inspect "$VOLUME" --format '{{ .Mountpoint }}' ;;
        *)
            echo "kernel.sh: 不支持的宿主平台 $(uname -s)" >&2; exit 2 ;;
    esac
}

docker_cmd() {
    # 通用容器调用：volume 挂 /ksrc（workspace），导出目录挂 /out
    docker run --rm -i ${DOCKER_TTY:-} --entrypoint= \
        -v "$VOLUME:/ksrc" \
        -v "$SCRIPT_DIR/../target/kernel:/out" \
        -w /ksrc \
        "$IMAGE" "$@"
}

ensure_image() {
    require_orbstack_engine
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        echo "kernel.sh: pulling $IMAGE ..." >&2
        if ! docker pull "$IMAGE" 2>/dev/null; then
            echo "kernel.sh: 拉取失败，回落本地构建 $SCRIPT_DIR/Dockerfile.kernel" >&2
            docker build -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.kernel" "$SCRIPT_DIR"
        fi
    fi
}

# macOS 引擎守卫：volume 的宿主视图由 OrbStack 提供，docker 端点必须指向
# OrbStack 引擎，否则 clone/build 会把卷建进别的引擎 VM，视图永远看不到。
require_orbstack_engine() {
    [ "$(uname -s)" = "Darwin" ] || return 0
    ep="${DOCKER_HOST:-$(docker context inspect --format '{{ .Endpoints.docker.Host }}' "$(docker context show)" 2>/dev/null || true)}"
    case "$ep" in
        *.orbstack*) ;;
        *)
            echo "kernel.sh: macOS 上当前 docker 引擎不是 OrbStack（endpoint: ${ep:-未知}）。" >&2
            echo "  执行 'docker context use orbstack'，或对单条命令 export DOCKER_HOST=unix://$HOME/.orbstack/run/docker.sock。" >&2
            exit 2 ;;
    esac
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
    docker volume create "$VOLUME" >/dev/null
    docker run --rm -i --entrypoint= -v "$VOLUME:/ksrc" "$IMAGE" \
        sh -c "git clone --depth 1 --branch '$REF' '$url' /tmp/k && \
               cp -a /tmp/k/. /ksrc/"
    # clangd 配置按目标架构落进源码根（compile_commands.json 的消费侧）
    docker run --rm -i --entrypoint= -v "$VOLUME:/ksrc" -v "$SCRIPT_DIR:/cfg:ro" "$IMAGE" \
        sh -c "sed 's/--target=[a-z0-9_-]*/--target=$(clangd_target)/' /cfg/.clangd > /ksrc/.clangd"
    echo "kernel.sh: cloned $url@$REF → volume ${VOLUME}（.clangd 已按 ${ARCH} 配好）"
    echo "kernel.sh: 宿主可见路径（AI/编辑器 cwd）：$(host_path 2>/dev/null || echo '(见 kernel.sh path)')"
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
    HOSTVIEW="$(host_path)"
    docker_cmd sh -c "$(make_env) make -j$JOBS Image modules && \
        python3 scripts/compile_commands.py && \
        sed -i 's|/ksrc|'"$HOSTVIEW"'|g' compile_commands.json"
    echo "kernel.sh: built Image + modules；compile_commands.json 已生成（宿主路径形态，宿主 clangd 直接消费）"
    ;;
cc)
    ensure_image
    HOSTVIEW="$(host_path)"
    docker_cmd sh -c "python3 scripts/compile_commands.py && \
        sed -i 's|/ksrc|'"$HOSTVIEW"'|g' compile_commands.json"
    ;;
ccr)
    # 容器 /ksrc 原始形态 CDB（devcontainer 内 clangd 消费）
    ensure_image
    docker_cmd python3 scripts/compile_commands.py
    ;;
ccfix)
    # 已有 CDB 原地改写为宿主路径形态（增量构建后 / ccr 之后的反向操作）
    ensure_image
    HOSTVIEW="$(host_path)"
    docker_cmd sh -c "sed -i 's|/ksrc|'"$HOSTVIEW"'|g' compile_commands.json"
    ;;
path)
    hp="$(host_path)"
    if [ ! -d "$hp" ]; then
        case "$(uname -s)" in
            Darwin)
                echo "kernel.sh: $hp 不可达——OrbStack 未安装或未运行（视图仅在 OrbStack 运行时存在）。" >&2
                echo "  装回/启动 OrbStack 后重试；或 'kernel.sh export' 走最小树回退（KERNEL_PATH 指 target/kernel/${ARCH}）。" >&2 ;;
            *)
                echo "kernel.sh: volume $VOLUME 不存在或不可达——先 'kernel.sh clone <git-url>'。" >&2 ;;
        esac
        exit 1
    fi
    echo "$hp"
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
    sed -n '2,37p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
