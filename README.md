# qemu-e2e — End-to-End Kernel Testing in QEMU

A small, hackable harness for booting a freshly-built Linux kernel under QEMU and running real test programs against it. Designed to drop into a kernel source tree as `qemu-e2e/` and give you a one-command answer to *"does my patch actually work?"*.

```bash
make qemu-test QEMU_TIMEOUT=30
# ...
# Test Results: 1/1 passed
# TEST_COMPLETE: ALL TESTS PASSED
```

## Why this exists

Most kernel iteration loops look like: edit → `make` → ??? → push to a real machine → maybe panic → pray for serial output. `qemu-e2e` collapses that into a hermetic loop:

- **Boots your built kernel directly** — no distro userland, no overlay images, no flashing hardware.
- **Statically-linked C tests** in a BusyBox rootfs — exercise syscalls, ioctls, `/proc`, `/sys`, `/dev` against the kernel under test.
- **Multi-architecture** out of the box — arm64, x86_64, riscv64.
- **Reproducible** — the same `.env` and the same kernel produce the same boot every time.
- **CI-friendly** — `make qemu-test QEMU_TIMEOUT=N` exits non-zero on test failure, kernel panic, or timeout. Drop it straight into a pipeline.
- **Debuggable** — KVM acceleration, GDB stub, interactive shell, optional NVMe block device, PCI passthrough hook.

It is intentionally *not* a distro builder, not a container runtime, and not a fuzzer. It's the smallest thing that lets a kernel patch and a test program meet.

## Setup

### 1. Build the kernel

```bash
git clone https://gitcode.com/openeuler/kernel.git && cd kernel
cp arch/arm64/configs/openeuler_defconfig .config   # or your own .config
make -j"$(nproc)"
make modules -j"$(nproc)"        # only if any module is =m
```

`qemu-e2e/` is meant to live *inside* this tree:

```bash
git clone https://gitcode.com/nashzhou/qemu-e2e.git
# result: kernel/qemu-e2e/
```

If you keep the harness elsewhere, set `KERNEL_PATH` in `.env` (see below).

### 2. Install host packages

```bash
# openEuler / CentOS / Fedora
sudo dnf install -y gcc make cmake wget cpio gzip qemu-system-aarch64 qemu-img

# Debian / Ubuntu
sudo apt install -y gcc make cmake wget cpio gzip qemu-system-arm qemu-utils
```

For x86_64 or riscv64 targets, install the matching `qemu-system-x86_64` / `qemu-system-riscv64`.

### 3. Configure and verify

```bash
cd qemu-e2e
cp .env.example .env             # edit if defaults don't fit
make verify                      # checks tools, kernel image, modules
make qemu-test QEMU_TIMEOUT=30
```

`make verify` is the fastest way to know if everything is wired up — it prints colored PASS/FAIL/WARN lines for every prerequisite.

## Configuration (`.env`)

`.env` is sourced by the Makefile and every shell script. `.env.example` is the tracked template; copy it to `.env` and edit. Relative paths resolve from the project root (`qemu-e2e/`).

| Variable | Default | Purpose |
|---|---|---|
| `KERNEL_PATH` | `..` (auto) | Kernel source tree. Override only for unusual layouts. |
| `ARCH` | `arm64` | Target architecture: `arm64`, `x86_64`, or `riscv64`. Picks QEMU binary, kernel image path, console device. |
| `QEMU_TIMEOUT` | `30` | Wallclock cap for `make qemu-test`. `0` is rejected. |
| `NUMA_MEMORY` | `1G` | Per-NUMA-node memory. Total = `NUMA_MEMORY` × `NUMA_NODES`. |
| `SMP` | `8` | Total vCPUs. Split evenly across NUMA nodes; must be divisible by `NUMA_NODES`. |
| `NUMA_NODES` | `2` | NUMA node count. `1` = single-node (no `-numa`). `>1` = one socket per node. |
| `QEMU` | auto | Override path to `qemu-system-<arch>`. |
| `QEMU_OPTS` | empty | Extra QEMU args, e.g. `-device vfio-pci,host=XX:XX.X`. |

### Architecture matrix

| `ARCH` | QEMU binary | Kernel image | Console |
|---|---|---|---|
| `arm64` (default) | `qemu-system-aarch64` | `arch/arm64/boot/Image` | `ttyAMA0` |
| `x86_64` | `qemu-system-x86_64` | `arch/x86/boot/bzImage` | `ttyS0` |
| `riscv64` | `qemu-system-riscv64` | `arch/riscv/boot/Image` | `ttyS0` |

Cross-compile freely (e.g. `ARCH=x86_64` on an arm64 host) — `make verify` warns when the target differs from the host.

## VM environment

| | |
|---|---|
| Memory | `NUMA_MEMORY` per node × `NUMA_NODES` (default 2 GB with `NUMA_NODES=2`) |
| CPUs | `SMP` total, split evenly across NUMA nodes (default 8) |
| NUMA | `NUMA_NODES` nodes, one socket per node (default 2) |
| Machine | `virt` (arm64/riscv64) / `q35` (x86_64) |
| Console | Serial only (`-nographic -serial mon:stdio`) |
| Block | `rootfs.img` ext4 system disk on virtio (`/dev/vda`); optional 512 MB NVMe (`disk.qcow2` → `/dev/nvme0n1`) |
| Userland | BusyBox 1.36.1, statically linked, per-arch binary cached under `infra/busybox/bin/` (prebuilt release download first, source-build fallback) |
| Cmdline | `console=<serial> root=/dev/vda rw init=/init loglevel=8 auto_test` |

Boot is two-stage:

1. **initramfs** (`initrd.img`, PID 1 = `infra/init-initramfs`) — one job: make `root=` mountable. Mounts `proc`/`sysfs`/`devtmpfs` (+ device-node fallbacks for kernels with quirky devtmpfs), insmods the **boot-critical** modules from `modules-boot.conf` (virtio + ext4 and deps), then mounts `root=` and `switch_root`s into it.
2. **rootfs** (`rootfs.img`, PID 1 = `infra/init`) — remounts pseudo-filesystems idempotently, insmods the **test** modules from `modules.conf` (now living in the rootfs at `/lib/modules/`), then either drops to a shell (interactive) or executes every binary in `/tests/` and powers off (auto-test).

## Makefile targets

| Target | Description |
|---|---|
| `make verify` | Validate prerequisites: `.env`, host tools, `KERNEL_PATH`, kernel image, QEMU, modules, BusyBox cache. |
| `make busybox` | Ensure the per-arch static BusyBox: prebuilt download from release first, source-build fallback. |
| `make initrd` | (Re)build the boot pair: `infra/initrd.img` (minimal initramfs + modules) and `infra/rootfs.img` (ext4 rootfs + tests). |
| `make qemu` | Boot interactively; lands in a BusyBox shell. |
| `make qemu-kvm` | Same, with KVM acceleration (host arch == target arch only). |
| `make qemu-debug` | Boot halted, with GDB stub on `:1234`. |
| `make qemu-test` | CI mode: rebuild initrd, run with `QEMU_TIMEOUT=N`, exit non-zero on failure or timeout. |
| `make disk` | Create `infra/disk.qcow2` (512 MB) for block-device tests. |
| `make install-skill` | Copy the `kernel-dev` Claude Code skill into `$KERNEL_PATH/.claude/skills/`. |
| `make uninstall-skill` | Remove it. |
| `make clean` | Remove `disk.qcow2`, `initrd.img`, `rootfs.img`, `testcases/build/`. |

## Directory layout

```
kernel/                              # Linux kernel source tree
└── qemu-e2e/                        # ← this framework
    ├── Makefile                     # top-level targets
    ├── .env.example                 # tracked config template
    ├── .env                         # your overrides (gitignored)
    ├── README.md
    ├── skills/
    │   └── kernel-dev/SKILL.md      # Claude Code skill (assistant guidance)
    └── infra/
        ├── verify.sh                # prerequisite checker
        ├── run-qemu.sh              # QEMU launcher (multi-arch / KVM / GDB / NVMe)
        ├── build-initrd.sh          # BusyBox + modules → initrd.img; BusyBox + tests → rootfs.img
        ├── init-initramfs           # stage-1 PID 1: insmod boot modules, mount root=, switch_root
        ├── modules-boot.conf        # boot-critical modules only (virtio, ext4 + deps) → initramfs
        ├── init                     # test PID 1 (injected into rootfs.img; auto_test or shell)
        ├── cpio2ext4.sh             # convert a release rootfs cpio.gz into an auto-sized ext4 img
        ├── modules.conf             # one module per line, optional load-time params
        ├── initrd.img               # generated, minimal initramfs
        ├── rootfs.img               # generated, ext4 rootfs
        ├── disk.qcow2               # generated, optional
        ├── initramfs/               # initramfs staging area (gitignored)
        ├── rootfs/                  # rootfs staging area (gitignored)
        └── testcases/
            ├── Makefile             # delegates to CMake
            ├── CMakeLists.txt       # one add_executable() per test binary
            └── src/
                ├── main.c           # shared entry — calls run_tests()
                ├── test_common.h/c  # PASS/FAIL/SKIP/INFO macros + counters
                └── test_example.c   # skeleton; copy and rename
```

## Writing a test case

Each test binary shares `main.c`, which prints a header, calls **`void run_tests(void)`** (which *you* implement), and prints a summary. Use the macros from `test_common.h` — they update the counters `main.c` reports.

### 1. Create `infra/testcases/src/test_<name>.c`

```c
#include "test_common.h"

static void test_my_feature(void)
{
    printf("\nTest: my feature does the thing\n");

    /* exercise the kernel via syscalls / ioctls / /proc / /sys */
    if (/* expected condition */)
        PASS("the thing happened");
    else
        FAIL("expected X, got Y");
}

void run_tests(void)
{
    test_my_feature();
    /* add more test_*() calls here */
}
```

Macros: `PASS(fmt, ...)`, `FAIL(fmt, ...)`, `SKIP(fmt, ...)`, `INFO(fmt, ...)`. Don't write your own `main()` — the shared one is linked in.

### 2. Register the binary in `infra/testcases/CMakeLists.txt`

```cmake
add_executable(test-<name>
    src/main.c
    src/test_common.c
    src/test_<name>.c
)
set_target_properties(test-<name> PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
```

The build is `-static -O2 -Wall`; do **not** relax `-static` — the VM has no dynamic loader.

### 3. Rebuild and run

```bash
make initrd && make qemu-test QEMU_TIMEOUT=30
```

`init` auto-discovers everything in `/tests/`, so newly-added binaries are picked up with no further wiring.

### Output markers

When auto-testing, look for:

- `[PASS] / [FAIL] / [SKIP] / [INFO]` — per-assertion lines.
- `Test Results: N/M passed` — per-binary tally.
- `TEST_COMPLETE: ALL TESTS PASSED` — overall success (harness exits 0).
- `TEST_COMPLETE: SOME TESTS FAILED` — overall failure (harness exits non-zero).
- Exit code `124` — harness wallclock timeout (kernel hang or runaway loop).

## Loading kernel modules

`infra/modules-boot.conf` (boot-critical: virtio/ext4) is copied into the initramfs and insmodded by `init-initramfs` before the pivot. `infra/modules.conf` (test modules, e.g. NVMe) is copied into `rootfs.img /lib/modules/` and insmodded by the test init after the pivot — adding a test module never changes `initrd.img`. Both: one module per line, `#` for comments, tokens after the name passed verbatim to `insmod`.

```
# dependencies first — there is no auto-resolution
nvme-core
nvme

# load-time parameters
my_driver param1=1 param2=foo
```

Rules:

- **Order matters.** `init` calls `insmod` in file order.
- **Module must be built.** A name in `modules.conf` without a matching `.ko` under `KERNEL_PATH` aborts the initrd build with `ERROR: Module X.ko not found`.
- **Prefer `=m` over `=y`** for modules under iteration — faster cycle, no kernel rebuild.

## Customizing the boot environment

`infra/init` is a small POSIX shell script. Edit `main()` between `load_modules` and the auto-test block to add setup the harness doesn't do by default. Example — pre-allocate huge pages and mount hugetlbfs:

```sh
mkdir -p /mnt/huge
mount -t hugetlbfs nodev /mnt/huge
echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

After editing `init` (test stage) or `init-initramfs` (pivot stage), rebuild the boot pair:

```bash
make initrd && make qemu-test QEMU_TIMEOUT=30
```

## Debugging

### Interactive shell

```bash
make qemu          # TCG, useful for cross-arch
make qemu-kvm      # native speed, host arch only
```

Exit with `Ctrl-A` then `x`.

### Source-level kernel debugging

Build the kernel with `CONFIG_DEBUG_INFO=y` (and ideally `CONFIG_DEBUG_INFO_DWARF5=y`, `CONFIG_GDB_SCRIPTS=y`).

Terminal 1:

```bash
make qemu-debug   # halts at boot, listening on :1234
```

Terminal 2:

```bash
cd "$KERNEL_PATH"
gdb-multiarch vmlinux -ex 'target remote :1234'
```

### Block device tests

```bash
make disk         # creates infra/disk.qcow2 (512 MB, NVMe)
make qemu         # appears as /dev/nvme0n1 in the VM
```

### PCI passthrough

```bash
QEMU_OPTS='-device vfio-pci,host=XX:XX.X' make qemu
```

Host needs IOMMU enabled (`intel_iommu=on` / `iommu=pt`).

## Claude Code integration

A `kernel-dev` skill lives under `skills/kernel-dev/SKILL.md` and teaches Claude Code how to drive this harness — running the dev loop, writing tests in the project's idiom, parsing serial output, and triaging failures.

```bash
make install-skill          # copies skill into $KERNEL_PATH/.claude/skills/
make uninstall-skill        # removes it
```

Once installed, Claude discovers the skill automatically when invoked from the kernel tree.

## Contributing

Issues and PRs welcome. When adding features, please:

- Keep the harness POSIX-shell-compatible where possible (`init` runs under BusyBox `sh`, not bash).
- Update `verify.sh` with any new prerequisite.
- Add a corresponding `test_*.c` for any new behavior the harness exposes.
- Update both this README and `skills/kernel-dev/SKILL.md` if user-visible behavior changes.

## License

See `LICENSE`.
