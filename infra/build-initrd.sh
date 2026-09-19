#!/bin/bash
# Build the two-piece boot image pair for two-stage boot:
#
#   initrd.img — minimal initramfs: busybox + ALL kernel modules + pivot
#                init (infra/init-initramfs). Loads every .ko from
#                modules.conf, mounts root=/dev/vda, switch_root.
#   rootfs.img — ext4 rootfs: busybox userland + test init (infra/init)
#                + /tests. This is the mutable working environment.
#
# Boot chain (see run-qemu.sh):
#   kernel → initramfs: insmod all → mount root → switch_root
#          → rootfs /init: auto_test or interactive shell
#
# Rootfs is a plain ext4 image: edit it (loop mount / cpio2ext4.sh) without
# touching the initramfs. Kernel modules live ONLY in the initramfs — they
# stay resident across switch_root, so the rootfs needs none.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
INITRAMFS_DIR="${SCRIPT_DIR}/initramfs"
ROOTFS_DIR="${SCRIPT_DIR}/rootfs"
INITRD_FILE="${SCRIPT_DIR}/initrd.img"
ROOTFS_IMG="${SCRIPT_DIR}/rootfs.img"

# Source .env from project root
if [ -f "${PROJECT_ROOT}/.env" ]; then
    set -a
    . "${PROJECT_ROOT}/.env"
    set +a
fi

# Auto-detect KERNEL_PATH (one level above qemu-e2e/)
KERNEL_PATH="${KERNEL_PATH:-$(cd "${PROJECT_ROOT}/.." && pwd)}"

# Verify kernel directory exists (needed for .ko files)
if [ ! -d "${KERNEL_PATH}/arch" ]; then
    echo "ERROR: Cannot find kernel source directory at ${KERNEL_PATH}"
    echo "Set KERNEL_PATH environment variable to specify the location:"
    echo "  KERNEL_PATH=/path/to/kernel $0"
    exit 1
fi

command -v mke2fs >/dev/null 2>&1 \
    || { echo "ERROR: mke2fs not found (install e2fsprogs)"; exit 1; }

# Architecture resolution (same logic as run-qemu.sh)
ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
    aarch64|arm64)  ARCH="arm64" ;;
    x86_64|amd64)   ARCH="x86_64" ;;
    riscv64)        ARCH="riscv64" ;;
    *) echo "ERROR: Unsupported ARCH=$ARCH"; exit 1 ;;
esac

# Ensure per-arch static BusyBox (release download first, source build fallback)
"${SCRIPT_DIR}/fetch-busybox.sh"
BUSYBOX_BIN="${SCRIPT_DIR}/busybox/bin/busybox-${ARCH}"

# Populate <dest> with the busybox userland: static binary + applet symlinks
# + skeleton dirs + root account. NOTE: applet installation executes the
# busybox binary, so cross-arch builds need a matching host (see fetch
# warning in fetch-busybox.sh).
assemble_busybox_tree() {
    local dest="$1"
    rm -rf "${dest}"
    mkdir -p "${dest}/bin" "${dest}/sbin" "${dest}/usr/bin" "${dest}/usr/sbin" \
             "${dest}/proc" "${dest}/sys" "${dest}/dev" "${dest}/tmp" \
             "${dest}/mnt" "${dest}/etc/init.d" "${dest}/var/run" "${dest}/root"

    cp "${BUSYBOX_BIN}" "${dest}/bin/busybox"
    (cd "${dest}/bin" && ./busybox --install -s .)

    # Rewrite absolute applet symlinks (busybox may point at /usr/bin/...)
    local link target
    for link in $(cd "${dest}" && find bin sbin usr/bin usr/sbin -type l); do
        target="$(readlink "${dest}/${link}")"
        if [[ "$target" == /* ]]; then
            ln -sf "$(basename "$target")" "${dest}/${link}"
        fi
    done

    printf 'root:x:0:0:root:/root:/bin/sh\n' > "${dest}/etc/passwd"
    printf 'root:x:0:\n' > "${dest}/etc/group"
}

# Copy kernel module <mod>.ko from the kernel tree into $1 (fallback: testcases/)
copy_module() {
    local dest="$1" mod="$2" ko_file
    ko_file="$(find ${KERNEL_PATH} -name "${mod}.ko" -print -quit 2>/dev/null)"
    [ -f "$ko_file" ] || ko_file="${SCRIPT_DIR}/testcases/${mod}.ko"
    if [ -f "$ko_file" ]; then
        cp "$ko_file" "${dest}/"
    else
        echo "ERROR: Module ${mod}.ko not found" >&2
        exit 1
    fi
}

# Copy every module listed in modules.conf into <dest> and keep the conf
copy_modules() {
    local dest="$1"
    mkdir -p "${dest}"
    local MODULES_CONF="${SCRIPT_DIR}/modules.conf"
    if [ -f "$MODULES_CONF" ]; then
        echo "Copying kernel modules..."
        local line mod_name
        while IFS= read -r line || [ -n "$line" ]; do
            [ -z "$line" ] && continue
            [ "${line#\#}" != "$line" ] && continue
            mod_name="${line%% *}"
            copy_module "${dest}" "$mod_name"
        done < "$MODULES_CONF"
        cp "$MODULES_CONF" "${dest}/modules.conf"
    else
        echo "  WARNING: modules.conf not found, booting without extra modules"
    fi
}

# Build testcases and copy binaries into <dest>/tests
install_testcases() {
    local dest="$1"
    echo "Building testcases..."
    mkdir -p "${dest}/tests"
    [ -d "${SCRIPT_DIR}/testcases" ] || { echo "  WARNING: no testcases dir"; return 0; }
    local log="${SCRIPT_DIR}/testcases/.build.log"
    (cd "${SCRIPT_DIR}/testcases" && make > "${log}" 2>&1) || {
        grep -E 'error:' "${log}" || cat "${log}"
        echo "ERROR: Testcases build failed"
        exit 1
    }
    grep -E 'warning:' "${log}" || true
    rm -f "${log}"
    if [ -d "${SCRIPT_DIR}/testcases/build/bin" ]; then
        local test_bin
        for test_bin in "${SCRIPT_DIR}/testcases/build/bin"/*; do
            if [ -f "$test_bin" ] && [ -x "$test_bin" ]; then
                cp "$test_bin" "${dest}/tests/"
                chmod +x "${dest}/tests/$(basename "$test_bin")"
                echo "  Test: $(basename "$test_bin")"
            fi
        done
    fi
}

# ---------- initrd.img: minimal initramfs ----------
echo "Building initrd.img (minimal initramfs)..."
assemble_busybox_tree "${INITRAMFS_DIR}"
cp "${SCRIPT_DIR}/init-initramfs" "${INITRAMFS_DIR}/init"
chmod 755 "${INITRAMFS_DIR}/init"
mkdir -p "${INITRAMFS_DIR}/mnt" "${INITRAMFS_DIR}/lib/modules"
copy_modules "${INITRAMFS_DIR}/lib/modules"

(cd "${INITRAMFS_DIR}" && find . -print0 | cpio --null -o -H newc 2>/dev/null) \
    | gzip -9 > "${INITRD_FILE}"

# ---------- rootfs.img: ext4 rootfs with tests ----------
echo "Building rootfs.img (ext4 rootfs)..."
assemble_busybox_tree "${ROOTFS_DIR}"
cp "${SCRIPT_DIR}/init" "${ROOTFS_DIR}/init"
chmod 755 "${ROOTFS_DIR}/init"
install_testcases "${ROOTFS_DIR}"

ROOTFS_SIZE_MB=$(( $(du -sm "${ROOTFS_DIR}" | cut -f1) + 2 ))
rm -f "${ROOTFS_IMG}"
mke2fs -q -F -t ext4 -L rootfs -d "${ROOTFS_DIR}" "${ROOTFS_IMG}" "${ROOTFS_SIZE_MB}M"

echo ""
echo "Done:"
ls -lh "${INITRD_FILE}" "${ROOTFS_IMG}"
