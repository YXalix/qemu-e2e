# CLAUDE.md — testing/

QEMU-based E2E testing framework for kernel feature validation. Boots a minimal VM with an initramfs and runs user-space test programs against the built kernel. Supports arm64, x86_64, and riscv64 via the `ARCH` env var.

## Quick Start

```bash
cd /root/kernel/testing
make qemu-test QEMU_TIMEOUT=30
```

All tests must complete within the timeout. For interactive debugging:

```bash
make qemu              # Interactive shell
make qemu-debug        # GDB stub on port 1234
make qemu-kvm          # KVM-accelerated
```

## Configuration

All settings live in `.env` (gitignored, personal overrides) with `.env.example` as the tracked template. Copy `.env.example` to `.env` and customize: `cp .env.example .env`. The Makefile and all shell scripts (`run-qemu.sh`, `build-initrd.sh`, `verify.sh`) auto-source `.env` on startup.

Relative paths in `.env` (e.g. `KERNEL_PATH=../..`) resolve relative to the project root (`qemu-e2e/`).

## Directory Layout

```
testing/
  .env.example            # Tracked config template (all variables with defaults)
  .env                    # Personal overrides (gitignored)
  Makefile                # Top-level targets: qemu, qemu-test, initrd, disk, clean
  infra/
    run-qemu.sh           # QEMU launcher (auto-detects binary, handles KVM/debug/auto-test)
    build-initrd.sh       # Builds initrd: BusyBox + modules + tests
    init                  # VM init script: mount fs, load modules, run tests
    modules.conf          # Kernel modules to copy into initrd and load at boot
    disk.qcow2            # Optional 512MB NVMe block device (make disk)
    rootfs/               # Build artifact — initramfs staging directory (gitignored)
    testcases/
      Makefile            # Wrapper: delegates to CMake
      CMakeLists.txt      # Static C test binaries, output to build/bin/
      src/
        main.c            # Test runner: prints kernel info, calls run_tests()
        test_common.h/c   # PASS/FAIL/SKIP macros and counters
        test_example.c    # Skeleton test — add new tests following this pattern
      build/              # CMake build output (gitignored)
```

## Architecture

| Component | Role |
|-----------|------|
| `Makefile` | Top-level orchestration; `qemu-test` target builds initrd then runs with timeout |
| `build-initrd.sh` | Downloads/builds BusyBox 1.36.1 (static), copies modules from `modules.conf`, builds tests via CMake, packs cpio+gzip initrd |
| `init` | PID 1 in VM: mounts proc/sys/dev/tmpfs/debugfs, loads modules, auto-runs `/tests/*` executables, powers off |
| `run-qemu.sh` | Configures QEMU: 1GB RAM, 8 CPUs, `virt`/`q35` machine, NVMe disk, serial console. Env vars: `QEMU`, `QEMU_KVM`, `QEMU_DEBUG`, `AUTO_TEST`, `QEMU_OPTS`, `ARCH` |
| `modules.conf` | One module per line, `#` comments. Modules are searched in kernel tree and copied as `.ko` files |

## VM Configuration

- **Architecture**: arm64 (default), x86_64, riscv64 — set via `ARCH` env var
- **Memory**: 1GB (`memory-backend-memfd`)
- **CPUs**: 8 (TCG or KVM `host`)
- **Console**: Serial (`ttyAMA0` on arm64, `ttyS0` on x86_64/riscv64), no graphics
- **Storage**: Optional 512MB NVMe SSD (`disk.qcow2`)
- **Kernel cmdline**: `console=<serial> root=/dev/ram0 rw=1 init=/init loglevel=8 auto_test`

## Adding a New Test

1. Create `testcases/src/test_<name>.c` implementing `void run_tests(void)` using `PASS()`/`FAIL()`/`SKIP()` macros from `test_common.h`
2. Add a new `add_executable(test-<name> ...)` target in `testcases/CMakeLists.txt`
3. Rebuild: `make initrd`
4. Run: `make qemu-test QEMU_TIMEOUT=30`

## Adding a Kernel Module

1. Add module name to `modules.conf` (one per line, no `.ko` suffix)
2. `build-initrd.sh` will find and copy it from the kernel build tree
3. The `init` script loads all listed modules at boot via `modprobe`/`insmod`

## Environment Variables

All variables can be set via `.env` (preferred), shell env vars, or Make overrides (e.g. `make qemu-test QEMU_TIMEOUT=60`).

| Variable | Default | Description |
|----------|---------|-------------|
| `KERNEL_PATH` | `../..` (relative to `infra/`) | Kernel source tree root |
| `QEMU` | auto-detect | Path to `qemu-system-<arch>` |
| `QEMU_OPTS` | empty | Extra QEMU arguments (e.g., PCI passthrough) |
