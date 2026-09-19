#!/bin/bash
# QEMU launch script for E2E testing

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

# Architecture detection (override with ARCH= env var)
ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
    aarch64|arm64)  ARCH="arm64"; QEMU_BIN_DEFAULT="qemu-system-aarch64"; CPU_TCG="cortex-a72"; CONSOLE_DEV="ttyAMA0"; KERNEL_IMG="arch/arm64/boot/Image" ;;
    x86_64|amd64)   ARCH="x86_64"; QEMU_BIN_DEFAULT="qemu-system-x86_64"; CPU_TCG="qemu64"; CONSOLE_DEV="ttyS0"; KERNEL_IMG="arch/x86/boot/bzImage" ;;
    riscv64)         ARCH="riscv64"; QEMU_BIN_DEFAULT="qemu-system-riscv64"; CPU_TCG="rv64"; CONSOLE_DEV="ttyS0"; KERNEL_IMG="arch/riscv/boot/Image" ;;
    *) echo "ERROR: Unsupported ARCH=$ARCH"; exit 1 ;;
esac

# Default kernel path (can be overridden by command line argument)
DEFAULT_KERNEL="${KERNEL_PATH}/${KERNEL_IMG}"
KERNEL="${1:-$DEFAULT_KERNEL}"
INITRD="${SCRIPT_DIR}/initrd.img"
ROOTFS="${SCRIPT_DIR}/rootfs.img"
DISK="${SCRIPT_DIR}/disk.qcow2"

# Default kernel if not provided
if [ ! -f "$KERNEL" ]; then
    echo "ERROR: Kernel image not found at $KERNEL"
    echo "Usage: $0 [path-to-kernel-image]"
    echo "Default: ${DEFAULT_KERNEL}"
    echo ""
    echo "Set KERNEL_PATH or ARCH to adjust:"
    echo "  KERNEL_PATH=/path/to/kernel ARCH=x86_64 $0"
    exit 1
fi

if [ ! -f "$INITRD" ]; then
    echo "ERROR: Initramfs not found at $INITRD"
    echo "Run: make initrd  (to build initrd.img)"
    exit 1
fi

if [ ! -f "$ROOTFS" ]; then
    echo "ERROR: Rootfs image not found at $ROOTFS"
    echo "Run: make initrd  (to build rootfs.img)"
    exit 1
fi

# System disk: ext4 rootfs on virtio (the initramfs switch_roots into it)
ROOTFS_OPT="-drive file=$ROOTFS,format=raw,if=virtio"

# Optional extra test disk - NVMe interface
DISK_OPT=""
if [ -f "$DISK" ]; then
    DISK_OPT="-blockdev driver=qcow2,file.driver=file,file.filename=$DISK,node-name=ssd0,discard=unmap,file.discard=unmap,file.locking=off "
    DISK_OPT+="-device nvme,drive=ssd0,serial=nvme-ssd-0"
fi

# Memory / CPU / NUMA configuration (overridable via .env)
# NUMA_MEMORY is per-NUMA-node; total guest memory = NUMA_MEMORY * NUMA_NODES.
NUMA_MEMORY="${NUMA_MEMORY:-1G}"
SMP="${SMP:-8}"
NUMA_NODES="${NUMA_NODES:-1}"

if [ "$NUMA_NODES" -gt 1 ]; then
    if [ $(( SMP % NUMA_NODES )) -ne 0 ]; then
        echo "ERROR: SMP ($SMP) not divisible by NUMA_NODES ($NUMA_NODES)"
        exit 1
    fi
    PER_NODE_CPUS=$(( SMP / NUMA_NODES ))

    # Multiply the numeric prefix; keep the unit suffix (G/M)
    case "$NUMA_MEMORY" in
        *G|*g) TOTAL_MEM="$(( ${NUMA_MEMORY%[Gg]} * NUMA_NODES ))G" ;;
        *M|*m) TOTAL_MEM="$(( ${NUMA_MEMORY%[Mm]} * NUMA_NODES ))M" ;;
        *)     TOTAL_MEM="$(( NUMA_MEMORY * NUMA_NODES ))" ;;
    esac

    # NUMA: align one socket per node so QEMU stops warning about
    # CPUs from the same socket being assigned to different nodes.
    SMP_TOPO="${SMP},sockets=${NUMA_NODES},cores=${PER_NODE_CPUS},threads=1"

    MEMORY_BACKEND=""
    NUMA_OPTS=""
    for i in $(seq 0 $((NUMA_NODES - 1))); do
        MEMORY_BACKEND+="-object memory-backend-memfd,id=mem${i},size=${NUMA_MEMORY},share=off "
        cpu_start=$(( i * PER_NODE_CPUS ))
        cpu_end=$(( cpu_start + PER_NODE_CPUS - 1 ))
        NUMA_OPTS+="-numa node,nodeid=${i},memdev=mem${i},cpus=${cpu_start}-${cpu_end} "
    done
    MACHINE="virt"
else
    MEMORY_BACKEND="-object memory-backend-memfd,id=mem,size=$NUMA_MEMORY,share=off"
    MACHINE="virt,memory-backend=mem"
    NUMA_OPTS=""
    TOTAL_MEM="$NUMA_MEMORY"
    SMP_TOPO="$SMP"
fi

# CPU configuration
if [ "${QEMU_KVM:-}" = "1" ]; then
    CPU="host"
    KVM_OPTS="-enable-kvm"
    ACCEL_STATUS="KVM"
else
    CPU="$CPU_TCG"
    KVM_OPTS=""
    ACCEL_STATUS="TCG"
fi

# Graphics (serial only for testing)
CONSOLE="-nographic -serial mon:stdio"

# Additional options
OPTIONS="-no-reboot"

# Debug mode: enable GDB stub and wait for connection
if [ -n "$QEMU_DEBUG" ]; then
    DEBUG_OPTS="-s -S"
    echo "  DEBUG: GDB stub enabled on port 1234"
    echo "  DEBUG: Waiting for GDB connection before starting..."
else
    DEBUG_OPTS=""
fi

# Determine auto-test mode
AUTO_TEST_FLAG=""
if [ "${AUTO_TEST:-}" = "1" ]; then
    AUTO_TEST_FLAG=" auto_test"
fi

# PCI device passthrough (optional)
# Usage: QEMU_OPTS="-device vfio-pci,host=XX:XX.X" ./run-qemu.sh

echo "=========================================="
echo "QEMU E2E Test Environment"
echo "=========================================="
echo "  Kernel: $KERNEL"
echo "  Initrd: $INITRD"
echo "  Rootfs: $ROOTFS (virtio system disk)"
if [ -n "$DISK_OPT" ]; then
    echo "  Disk:   $DISK (NVMe test disk)"
fi
echo "  Memory: $TOTAL_MEM (per-node: $NUMA_MEMORY x $NUMA_NODES)"
echo "  CPUs: $SMP"
echo "  NUMA nodes: $NUMA_NODES"
echo "  Accelerator: $ACCEL_STATUS"
echo "  Auto-test: ${AUTO_TEST:-0}"
if [ -n "$QEMU_DEBUG" ]; then
    echo "  Debug: enabled (GDB port 1234)"
fi
echo ""
echo "Starting QEMU..."
echo "=========================================="

# QEMU binary - use QEMU env var or auto-detect
if [ -n "$QEMU" ]; then
    QEMU_BIN="$QEMU"
elif command -v "$QEMU_BIN_DEFAULT" >/dev/null 2>&1; then
    QEMU_BIN="$QEMU_BIN_DEFAULT"
else
    echo "ERROR: $QEMU_BIN_DEFAULT not found"
    echo "Set QEMU environment variable to specify the path:"
    echo "  QEMU=/path/to/$QEMU_BIN_DEFAULT $0"
    exit 1
fi

# Launch QEMU
set +e

$QEMU_BIN \
    -machine $MACHINE \
    $MEMORY_BACKEND \
    $NUMA_OPTS \
    $KVM_OPTS \
    -cpu $CPU \
    -smp $SMP_TOPO \
    -m $TOTAL_MEM \
    -kernel "$KERNEL" \
    -initrd "$INITRD" \
    -append "console=$CONSOLE_DEV root=/dev/vda rw init=/init loglevel=8${AUTO_TEST_FLAG}" \
    $ROOTFS_OPT \
    $DISK_OPT \
    ${QEMU_OPTS:-} \
    $CONSOLE \
    $OPTIONS \
    $DEBUG_OPTS

exit $?
