#!/bin/bash
# Build initrd.img from scratch

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOTFS_DIR="${SCRIPT_DIR}/rootfs"
INITRD_FILE="${SCRIPT_DIR}/initrd.img"
BUSYBOX_VERSION="1.36.1"
BUSYBOX_REPO="https://gitcode.com/gh_mirrors/bu/busybox.git"
BUSYBOX_BRANCH="1_36_stable"
BUSYBOX_DIR="${SCRIPT_DIR}/busybox/busybox"

# Source .env from project root
if [ -f "${PROJECT_ROOT}/.env" ]; then
    set -a
    . "${PROJECT_ROOT}/.env"
    set +a
fi

# Auto-detect KERNEL_PATH (one level above qemu-e2e/)
KERNEL_PATH="${KERNEL_PATH:-$(cd "${PROJECT_ROOT}/.." && pwd)}"

# Verify kernel directory exists
if [ ! -d "${KERNEL_PATH}/arch" ]; then
    echo "ERROR: Cannot find kernel source directory at ${KERNEL_PATH}"
    echo "Set KERNEL_PATH environment variable to specify the location:"
    echo "  KERNEL_PATH=/path/to/kernel $0"
    exit 1
fi

echo "Building initrd.img..."

# Create rootfs directory
rm -rf "${ROOTFS_DIR}"
mkdir -p "${ROOTFS_DIR}"
cd "${ROOTFS_DIR}"

# Create directories
mkdir -p bin sbin lib lib64 usr/bin usr/sbin proc sys dev tmp mnt \
         etc/init.d var/run root

# Build BusyBox if needed
if [ ! -f "${BUSYBOX_DIR}/busybox" ]; then
    echo "Building BusyBox..."
    mkdir -p "${SCRIPT_DIR}/busybox"
    cd "${SCRIPT_DIR}/busybox"

    if [ ! -d "${BUSYBOX_DIR}" ]; then
        git clone --depth 1 --branch "${BUSYBOX_BRANCH}" "${BUSYBOX_REPO}" "${BUSYBOX_DIR}"
    fi

    cd "${BUSYBOX_DIR}"
    make defconfig
    sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
    make -j$(nproc)
fi

# Copy BusyBox
cp "${BUSYBOX_DIR}/busybox" "${ROOTFS_DIR}/bin/"
cd "${ROOTFS_DIR}/bin"
./busybox --install -s .

# Fix symlinks
for link in $(find . -type l); do
    target=$(readlink "$link")
    if [[ "$target" == /* ]]; then
        ln -sf "$(basename "$target")" "$link"
    fi
done

cd "${ROOTFS_DIR}"

# Copy init script
cp "${SCRIPT_DIR}/init" .
chmod +x init

# Copy kernel module to lib/modules if found
# Usage: copy_module <module_name>
copy_module() {
    local mod="$1"
    local ko_file

    ko_file="$(find ${KERNEL_PATH} -name "${mod}.ko" -print -quit 2>/dev/null)"

    # Fallback to testcases directory
    if [ ! -f "$ko_file" ]; then
        ko_file="${SCRIPT_DIR}/testcases/${mod}.ko"
    fi

    if [ -f "$ko_file" ]; then
        cp "$ko_file" lib/modules/
    else
        echo "ERROR: Module ${mod}.ko not found" >&2
        exit 1
    fi
}

# Copy kernel modules from modules.conf
echo "Copying kernel modules..."
mkdir -p lib/modules

MODULES_CONF="${SCRIPT_DIR}/modules.conf"
if [ -f "$MODULES_CONF" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        # Skip comments and empty lines
        [ -z "$line" ] && continue
        [ "${line#\#}" != "$line" ] && continue
        # Extract module name (first whitespace-separated token);
        # any trailing tokens are module params consumed at load time.
        mod_name="${line%% *}"
        copy_module "$mod_name"
    done < "$MODULES_CONF"
    # Copy modules.conf into initrd for runtime use
    cp "$MODULES_CONF" lib/modules/modules.conf
else
    echo "  WARNING: modules.conf not found, skipping modules"
fi

# Build and copy testcases
echo "Building testcases..."
mkdir -p tests
if [ -d "${SCRIPT_DIR}/testcases" ]; then
    cd "${SCRIPT_DIR}/testcases"
    # Build using Makefile (which wraps CMake)
    # Only show warnings and errors; bin list is printed below
    make 2>&1 | grep -E 'warning:|error:' || true
    if [ "${PIPESTATUS[0]}" -ne 0 ]; then
        echo "ERROR: Testcases build failed"
        exit 1
    fi
    cd "${ROOTFS_DIR}"

    # Copy from build/bin directory
    if [ -d "${SCRIPT_DIR}/testcases/build/bin" ]; then
        for test_bin in "${SCRIPT_DIR}/testcases/build/bin"/*; do
            if [ -f "$test_bin" ] && [ -x "$test_bin" ]; then
                cp "$test_bin" tests/
                chmod +x "tests/$(basename "$test_bin")"
                echo "  Test: $(basename "$test_bin")"
            fi
        done
    fi
fi

# Build initrd
echo "Creating initrd.img..."
cd "${ROOTFS_DIR}"
find . -print0 | cpio --null -o -H newc 2>/dev/null | gzip -9 > "${INITRD_FILE}"

SIZE=$(ls -lh "${INITRD_FILE}" | awk '{print $5}')
echo "Done: ${INITRD_FILE} (${SIZE})"
