#!/bin/bash
# Convert a rootfs initramfs (cpio.gz from the busybox-v* release) into an
# ext4 disk image for qemu -drive booting.
#
# 用法：
#   cpio2ext4.sh <rootfs.cpio.gz> [out.img] [size]
#     out.img  默认 <src>.ext4.img（同目录）
#     size     镜像大小，如 16M；默认 auto —— 按解包后内容 +2MB 余量取整
#
# 依赖：gzip cpio mke2fs(e2fsprogs)。镜像大小按内容动态生成，
# 不会再出现 CI 里拍固定 64M 的浪费。
#
# 启动示例（root= 按实际 virtio 块设备名调整）：
#   qemu-system-aarch64 -M virt -nographic -kernel Image \
#     -drive file=rootfs.ext4.img,format=raw,if=virtio \
#     -append "console=ttyAMA0 root=/dev/vda rw init=/init"

set -e

usage() { grep '^# 用法' -A 3 "$0" | sed 's/^# \?//'; exit 1; }

[ $# -ge 1 ] || usage
SRC="$1"
[ -f "$SRC" ] || { echo "ERROR: $SRC not found"; exit 1; }

OUT="${2:-${SRC%.cpio.gz}.ext4.img}"
SIZE="${3:-auto}"

command -v mke2fs >/dev/null || { echo "ERROR: mke2fs missing (install e2fsprogs)"; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

gzip -dc "$SRC" | (cd "$TMP" && cpio -id --quiet)

if [ "$SIZE" = "auto" ]; then
    used_mb="$(du -sm "$TMP" | cut -f1)"
    SIZE="$((used_mb + 2))M"
fi

rm -f "$OUT"
mke2fs -q -F -t ext4 -L rootfs -d "$TMP" "$OUT" "$SIZE"

ls -lh "$OUT"
echo "Done: $OUT ($SIZE)"
