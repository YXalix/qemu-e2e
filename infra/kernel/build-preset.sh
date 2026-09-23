#!/usr/bin/env bash
# mainline preset 内核构建 —— defconfig + fragment（全 =y 零模块），三架构。
#
# 产出（$2 输出目录）：
#   Image-<arch>      可引导内核镜像（arm64/riscv64 = Image，x86_64 = bzImage）
#   config-<arch>     合并 fragment 后的最终 .config（可复现性凭证）
#   SHA256SUMS        全部资产的校验和
#
# 用法：build-preset.sh <版本，如 6.12.8> <输出目录> [内核源码目录]
# 源码目录缺省在脚本旁克隆（浅克隆 torvalds 树 v<版本> tag）；已存在则复用。
#
# 宿主依赖：git + 三架构编译器（native gcc + gcc-aarch64-linux-gnu +
# gcc-riscv64-linux-gnu）；CI（kernel-release.yml）负责安装，本脚本不越权。

set -euo pipefail

VER="${1:?用法: build-preset.sh <版本> <输出目录> [内核源码目录]}"
OUT="${2:?缺少输出目录}"
SRC="${3:-$(cd "$(dirname "$0")" && pwd)/linux-$VER}"
HERE="$(cd "$(dirname "$0")" && pwd)"

if [ ! -d "$SRC" ]; then
    git clone --depth 1 --branch "v$VER" \
        https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git "$SRC"
fi

mkdir -p "$OUT"

# build <arch> <资产名> <镜像相对路径> <CROSS_COMPILE 前缀，空 = native>
build() {
    local arch="$1" name="$2" img="$3" cross="$4"
    local o="$SRC/build-$arch"
    make -C "$SRC" O="build-$arch" ARCH="$arch" ${cross:+CROSS_COMPILE="$cross"} defconfig
    "$SRC/scripts/kconfig/merge_config.sh" -m -O "$o" "$o/.config" "$HERE/fragment.config"
    make -C "$SRC" O="build-$arch" ARCH="$arch" ${cross:+CROSS_COMPILE="$cross"} -j"$(nproc)" olddefconfig
    make -C "$SRC" O="build-$arch" ARCH="$arch" ${cross:+CROSS_COMPILE="$cross"} -j"$(nproc)" "$img"
    cp "$o/$img" "$OUT/Image-$name"
    cp "$o/.config" "$OUT/config-$name"
    echo "built: Image-$name ($(du -h "$OUT/Image-$name" | cut -f1))"
}

build arm64 arm64 arch/arm64/boot/Image aarch64-linux-gnu-
build x86_64 x86_64 arch/x86/boot/bzImage ""
build riscv64 riscv64 arch/riscv/boot/Image riscv64-linux-gnu-

( cd "$OUT" && sha256sum Image-* config-* > SHA256SUMS )
echo "done: $OUT"
