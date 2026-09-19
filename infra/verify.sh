#!/bin/bash
# Verify qemu-e2e prerequisites before building/running tests
# Usage: ./verify.sh [--quiet]

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Source .env from project root
if [ -f "${PROJECT_ROOT}/.env" ]; then
    set -a
    . "${PROJECT_ROOT}/.env"
    set +a
fi

# Auto-detect KERNEL_PATH (one level above qemu-e2e/)
KERNEL_PATH="${KERNEL_PATH:-$(cd "${PROJECT_ROOT}/.." && pwd)}"

QUIET=false

if [ "${1:-}" = "--quiet" ]; then
    QUIET=true
fi

# Architecture detection (same logic as run-qemu.sh)
ARCH="${ARCH:-$(uname -m)}"
HOST_ARCH="$(uname -m)"
case "$ARCH" in
    aarch64|arm64)  ARCH="arm64"; QEMU_BIN_DEFAULT="qemu-system-aarch64"; CPU_TCG="cortex-a72"; KERNEL_IMG="arch/arm64/boot/Image" ;;
    x86_64|amd64)   ARCH="x86_64"; QEMU_BIN_DEFAULT="qemu-system-x86_64"; CPU_TCG="qemu64"; KERNEL_IMG="arch/x86/boot/bzImage" ;;
    riscv64)         ARCH="riscv64"; QEMU_BIN_DEFAULT="qemu-system-riscv64"; CPU_TCG="rv64"; KERNEL_IMG="arch/riscv/boot/Image" ;;
    *) echo "ERROR: Unsupported ARCH=$ARCH"; exit 1 ;;
esac

# Counters
CRITICAL_PASS=0
CRITICAL_FAIL=0
WARNINGS=0
INFOS=0

# Color helpers (only when stdout is a terminal)
if [ -t 1 ]; then
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    RED='\033[0;31m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' YELLOW='' RED='' CYAN='' BOLD='' RESET=''
fi

log_pass() {
    CRITICAL_PASS=$((CRITICAL_PASS + 1))
    $QUIET && return
    echo -e "  ${GREEN}[PASS]${RESET} $1"
}

log_fail() {
    CRITICAL_FAIL=$((CRITICAL_FAIL + 1))
    echo -e "  ${RED}[FAIL]${RESET} $1"
}

log_warn() {
    WARNINGS=$((WARNINGS + 1))
    echo -e "  ${YELLOW}[WARN]${RESET} $1"
}

log_info() {
    INFOS=$((INFOS + 1))
    $QUIET && return
    echo -e "  ${CYAN}[INFO]${RESET} $1"
}

#==========================================
# Header
#==========================================
$QUIET || echo -e "${BOLD}[VERIFY] QEMU E2E Prerequisites Check${RESET}"
$QUIET || echo "========================================"

#==========================================
# 1. Configuration
#==========================================
if [ -f "${PROJECT_ROOT}/.env" ]; then
    log_pass "Configuration: .env found"
else
    log_fail "Configuration: .env not found (using .env.example defaults, copy it: cp .env.example .env)"
fi

#==========================================
# 2. Host tools
#==========================================
HOST_TOOLS="wget tar make cmake cpio gzip nproc find sed timeout"
MISSING_TOOLS=""

for tool in $HOST_TOOLS; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        MISSING_TOOLS="$MISSING_TOOLS $tool"
    fi
done

# gcc or cc
if command -v gcc >/dev/null 2>&1; then
    CC_NAME="gcc"
elif command -v cc >/dev/null 2>&1; then
    CC_NAME="cc"
else
    MISSING_TOOLS="$MISSING_TOOLS gcc/cc"
    CC_NAME=""
fi

if [ -z "$MISSING_TOOLS" ]; then
    log_pass "Host tools: all found ($HOST_TOOLS $CC_NAME)"
else
    log_fail "Host tools: missing -$MISSING_TOOLS"
fi

#==========================================
# 3. KERNEL_PATH
#==========================================
if [ -z "${KERNEL_PATH:-}" ]; then
    log_fail "KERNEL_PATH not set (create .env from .env.example)"
else
    log_pass "KERNEL_PATH: ${KERNEL_PATH}"
fi

#==========================================
# 4. Kernel source tree
#==========================================
if [ -d "${KERNEL_PATH}/arch" ]; then
    KVER=""
    if [ -f "${KERNEL_PATH}/Makefile" ]; then
        KVER="$(awk -F'= ' '/^VERSION|^PATCHLEVEL|^SUBLEVEL/{printf "%s.",$2}' "${KERNEL_PATH}/Makefile" | sed 's/\.$//')"
    fi
    log_pass "Kernel source: ${KERNEL_PATH}${KVER:+ (v${KVER})}"
else
    log_fail "Kernel source: ${KERNEL_PATH}/arch not found (set KERNEL_PATH)"
fi

#==========================================
# 5. Kernel image
#==========================================
KERNEL_IMAGE="${KERNEL_PATH}/${KERNEL_IMG}"
if [ -f "$KERNEL_IMAGE" ]; then
    KSIZE=$(ls -lh "$KERNEL_IMAGE" | awk '{print $5}')
    log_pass "Kernel image: ${KERNEL_IMG} (${KSIZE})"
else
    log_fail "Kernel image: ${KERNEL_IMG} not found (build the kernel first)"
fi

#==========================================
# 6. QEMU binary
#==========================================
if [ -n "${QEMU:-}" ]; then
    if command -v "$QEMU" >/dev/null 2>&1; then
        log_pass "QEMU binary: $QEMU (from env)"
    else
        log_fail "QEMU binary: $QEMU not found (from QEMU env var)"
    fi
elif command -v "$QEMU_BIN_DEFAULT" >/dev/null 2>&1; then
    log_pass "QEMU binary: $QEMU_BIN_DEFAULT"
else
    log_fail "QEMU binary: $QEMU_BIN_DEFAULT not found (install qemu-system-${ARCH})"
fi

# qemu-img (optional, for disk creation)
if command -v qemu-img >/dev/null 2>&1; then
    $QUIET || log_info "qemu-img: available (for 'make disk')"
else
    $QUIET || log_info "qemu-img: not found (optional, for disk image creation)"
fi

#==========================================
# 7. Kernel modules
#==========================================
MODULES_CONF="${SCRIPT_DIR}/modules.conf"
if [ -f "$MODULES_CONF" ]; then
    MOD_FOUND=0
    MOD_MISSING=""
    MOD_TOTAL=0

    while IFS= read -r line; do
        [ -z "$line" ] && continue
        [ "${line#\#}" != "$line" ] && continue
        MOD_TOTAL=$((MOD_TOTAL + 1))

        if find "${KERNEL_PATH}" -name "${line}.ko" -print -quit 2>/dev/null | grep -q .; then
            MOD_FOUND=$((MOD_FOUND + 1))
        else
            MOD_MISSING="$MOD_MISSING $line"
        fi
    done < "$MODULES_CONF"

    if [ "$MOD_FOUND" -eq "$MOD_TOTAL" ]; then
        log_pass "Kernel modules: ${MOD_FOUND}/${MOD_TOTAL} found"
    else
        log_warn "Kernel modules: ${MOD_FOUND}/${MOD_TOTAL} found, missing:${MOD_MISSING}"
    fi
else
    log_warn "modules.conf not found at ${MODULES_CONF}"
fi

#==========================================
# 8. BusyBox
#==========================================
BUSYBOX_BIN="${SCRIPT_DIR}/busybox/bin/busybox-${ARCH}"

if [ -x "$BUSYBOX_BIN" ]; then
    log_pass "BusyBox: cached (${ARCH})"
else
    log_info "BusyBox: not cached for ${ARCH} (release download on first build, source build fallback)"
fi

#==========================================
# 9. Cross-compile warning
#==========================================
HOST_NORMALIZED="$HOST_ARCH"
case "$HOST_NORMALIZED" in
    aarch64) HOST_NORMALIZED="arm64" ;;
    amd64)   HOST_NORMALIZED="x86_64" ;;
esac

if [ "$ARCH" != "$HOST_NORMALIZED" ]; then
    log_warn "Cross-compile: ARCH=$ARCH differs from host ($HOST_ARCH), ensure cross-toolchain is available"
fi

#==========================================
# 10. Disk image
#==========================================
DISK="${SCRIPT_DIR}/disk.qcow2"
if [ -f "$DISK" ]; then
    log_info "Disk image: ${DISK} exists"
else
    log_info "Disk image: not present (run 'make disk' to create)"
fi

#==========================================
# 11. initrd
#==========================================
INITRD="${SCRIPT_DIR}/initrd.img"
if [ -f "$INITRD" ]; then
    ISIZE=$(ls -lh "$INITRD" | awk '{print $5}')
    log_info "Initrd: ${INITRD} (${ISIZE})"
else
    log_info "Initrd: not built yet (run 'make initrd')"
fi

#==========================================
# Summary
#==========================================
$QUIET || echo "========================================"

if [ "$CRITICAL_FAIL" -gt 0 ]; then
    echo -e "  ${RED}${BOLD}FAIL: ${CRITICAL_FAIL} critical check(s) failed${RESET}"
    echo ""
    echo "  Fix the issues above, then run: make verify"
    exit 1
else
    echo -e "  ${GREEN}${BOLD}PASS: ${CRITICAL_PASS} critical checks OK${RESET}"
    [ "$WARNINGS" -gt 0 ] && echo -e "  ${YELLOW}WARN: ${WARNINGS} warning(s)${RESET}"
    echo ""
    echo "  Ready. Run: make qemu-test QEMU_TIMEOUT=30"
    exit 0
fi
