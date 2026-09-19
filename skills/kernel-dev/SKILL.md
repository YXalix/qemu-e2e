---
name: kernel-dev
description: Expert assistant for Linux kernel feature development and E2E verification through the in-tree qemu-e2e harness. Use when writing or debugging kernel code, adding kernel-side tests, diagnosing boot/panic failures, or verifying that a patch actually works in a booted VM.
user_invocable: true
version: 2.0.0
---

## Core Mission & Persona

You are an Expert Linux Kernel Developer and Systems Engineer. Your job is to help the user write, debug, and modify kernel code, and then strictly verify those changes by booting the kernel under QEMU via the `qemu-e2e/` harness. **Verification is not optional**: a feature is not "done" until a test inside the VM exercises it and the run prints `TEST_COMPLETE: ALL TESTS PASSED`.

## Triggering & Context

Activate this skill whenever the user asks to:
* Implement, modify, or debug a Linux kernel feature.
* Verify a kernel patch by booting and running tests.
* Add a new test case (`test_<name>.c`) under `qemu-e2e/infra/testcases/src/`.
* Add or reorder kernel modules loaded at boot (`infra/modules.conf`).
* Diagnose a kernel panic, boot hang, module load failure, or test-binary failure observed in QEMU serial output.
* Switch target architecture (arm64 / x86_64 / riscv64) or use KVM / GDB.

## Repository Layout

`qemu-e2e/` lives **inside** the kernel source tree. All paths in this skill are relative to that tree.

```
kernel/                              # kernel source root
├── arch/ mm/ fs/ drivers/ ...       # kernel code
└── qemu-e2e/                        # this harness
    ├── Makefile                     # thin forwarder to cargo xtask
    ├── virtuoso.toml                # single config surface: globals + [components.*]
    └── infra/
        ├── init                     # PID 1 inside VM: mount → insmod modules.conf → run /tests/*
        ├── modules-boot.conf        # frozen boot-critical module set → initramfs
        └── testcases/
            ├── CMakeLists.txt       # one add_executable() per test binary
            └── src/
                ├── main.c           # shared entry point, calls run_tests()
                ├── test_common.h/c  # PASS/FAIL/SKIP/INFO macros + counters
                └── test_<name>.c    # YOU ADD THESE (define void run_tests(void))
```

## Configuration (`virtuoso.toml`)

`virtuoso.toml` at the project root is the single config surface. The tracked
template ships the default regular-boot config active; every optional setting
is present as a comment. Unknown keys and bad types are rejected at parse time.
Key global keys:

| Key | Purpose | Notes |
|---|---|---|
| `kernel_path` | Kernel source root | Auto-detected as `..` from the harness dir; override only for unusual layouts |
| `arch` | Target arch | `arm64` (default), `x86_64`, `riscv64` — picks QEMU binary, kernel image path, console device |
| `timeout_secs` | `test` wallclock cap (s) | `0` is rejected; pick 30–120 for CI, longer if KVM is off and tests are heavy |
| `smp` | Total vCPUs | Must be divisible by the NUMA node count when > 1 |
| `backend` | `qemu` or `firecracker` | firecracker = microVM (x86_64/aarch64 + KVM) |
| `qemu_opts` | Extra QEMU args (array) | Escape hatch; passthrough components below are preferred |

VM capabilities are **components** under `[components.*]`, each with `enabled`,
`require` (kernel modules it needs) and `stage` (`boot` before root mount /
`runtime` after pivot, default `runtime`): `[components.tools_disk]` (tools
data disk, default on), `[components.agent]` (AI probe virtio-serial channel),
`[components.vfio]` (PCI passthrough via `devices`), `[components.numa]`
(multi-node topology via `nodes` / `memory_per_node`). The builder generates
the rootfs module list from the enabled components' `require` union.

A legacy `.env` is still read with a deprecation WARN; component config only
exists in TOML.

## Primary Workflow: The Kernel Dev Loop

When the user changes kernel code and wants verification, execute this loop end-to-end. **Do not skip the verify step** — it catches missing modules, missing kernel images, and toolchain gaps before you waste a build cycle.

1. **Verify prerequisites** — `cargo xtask verify` (run inside the harness dir)
   - Confirms `virtuoso.toml` exists, host tools present, `kernel_path` resolves, kernel image built, QEMU installed, every module required by enabled components is findable, and warns on cross-compile mismatches.
   - On failure, fix the reported gap before proceeding.

2. **Build the kernel** (in the kernel tree root, not `qemu-e2e/`)
   ```bash
   make -j"$(nproc)"
   make modules -j"$(nproc)"     # only if any module in modules.conf is =m
   ```
   The kernel image lands at the arch-specific path `verify` already validated:
   - arm64 → `arch/arm64/boot/Image`
   - x86_64 → `arch/x86/boot/bzImage`
   - riscv64 → `arch/riscv/boot/Image`

3. **Ensure test coverage** — does an existing `test_*.c` exercise the new behavior?
   - Yes → proceed.
   - No → add one (see "Adding a Test Case" below) **before** running the harness. A green run with no relevant assertions is worthless.

4. **Build the initrd** — `make -C qemu-e2e initrd`
   - Builds BusyBox 1.36.1 (cached after first run), copies every module from `modules.conf` (failing loudly on missing `.ko`), CMake-builds tests, packs `target/artifacts/initrd.img`.

5. **Run the verification** — `make -C qemu-e2e qemu-test QEMU_TIMEOUT=30`
   - Boots QEMU with serial console to stdout, kernel cmdline includes `auto_test`, init runs every binary in `/tests/`, then powers off.
   - Bump the timeout if reclaim/swap-heavy tests take longer; the harness kills the VM at the cap and exits 124.

6. **Analyze the output** — search the serial log for these markers:
   - `[PASS] / [FAIL] / [SKIP] / [INFO]` — per-assertion lines from `test_common.h`.
   - `Test Results: N/M passed` — per-binary tally from `init`.
   - `TEST_COMPLETE: ALL TESTS PASSED` → success. Anything else (`SOME TESTS FAILED`, kernel panic, `Kernel panic - not syncing`, `Unable to handle kernel paging request`, `BUG:`, `WARNING:`) → failure; report stack trace and the kernel file/line if visible.
   - Exit code 124 → harness timeout (often a kernel hang or runaway loop, not a test failure).

## Adding a Test Case

Tests are statically-linked C binaries that run as user-space inside the VM. The shared `main.c` prints a header, calls **`void run_tests(void)`** (which you define), then prints a summary. Each test binary has its own `run_tests()`.

1. **Create** `qemu-e2e/infra/testcases/src/test_<name>.c`:
   ```c
   #include "test_common.h"

   static void test_my_feature(void)
   {
       printf("\nTest: my feature does the thing\n");
       /* ... exercise kernel via syscalls, /proc, /sys, /dev, ioctls ... */
       if (/* expected condition */) PASS("the thing happened");
       else                          FAIL("expected X, got Y");
   }

   void run_tests(void)
   {
       test_my_feature();
       /* add more test_*() calls here */
   }
   ```
   Use **only** `PASS()`, `FAIL()`, `SKIP()`, `INFO()` from `test_common.h`. They increment the counters `main.c` reports. Do not implement your own `main()` — the shared one is linked in.

2. **Register** the binary in `qemu-e2e/infra/testcases/CMakeLists.txt`:
   ```cmake
   add_executable(test-<name>
       src/main.c
       src/test_common.c
       src/test_<name>.c
   )
   set_target_properties(test-<name> PROPERTIES
       RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
   ```
   Build flags (`-static -O2 -Wall`) come from the top of `CMakeLists.txt`; do not relax `-static` — the VM has no dynamic loader.

3. **Rebuild and run** — `make -C qemu-e2e initrd && make -C qemu-e2e qemu-test QEMU_TIMEOUT=30`. The init script auto-runs every executable in `/tests/`, so newly added tests are picked up with no further wiring.

### Test design rules

- **Verify kernel state, not user-space wrappers.** Read `/proc/<pid>/smaps`, `/sys/kernel/mm/...`, `/proc/meminfo`, `/proc/swaps` etc. to confirm the kernel actually did the thing.
- **Print intent before assertions.** A `printf("\nTest: ...\n")` header makes serial logs greppable when something panics mid-test.
- **Fail loudly with context.** `FAIL("expected nr_huge=10, got %d", nr)` beats `FAIL("wrong count")`.
- **Use SKIP for unsupported configs**, not FAIL — e.g. when the running kernel lacks the CONFIG the test exercises.

## Adding / Reordering Kernel Modules

`infra/modules.conf` is read by the builder (`cargo xtask build`, to copy `.ko` files) and `init` (to `insmod` them at boot). Format: one module per line, `#` for comments. Tokens after the module name are passed verbatim to `insmod`.

```
# dependency order is enforced by you; the harness does NOT topologically sort
nvme-core
nvme
my_driver param1=1 param2=foo
```

Rules:
- **List dependencies before dependents.** `init` calls `insmod` in file order; `modprobe`-style auto-resolution does not happen.
- **Module must be built.** If a name in `modules.conf` doesn't have a matching `.ko` in `KERNEL_PATH`, `cargo xtask build` aborts with `ERROR: Module X.ko not found`. Either build it (`make modules`) or remove the entry.
- **Prefer `=m` over `=y`** for modules under test — easier to iterate without rebuilding the kernel image.

## Customizing the Boot Environment

`infra/init` is the PID 1 shell script. Edit it when a feature needs setup the harness doesn't do by default — pre-allocating huge pages, mounting hugetlbfs, configuring zram, sysctl tuning, swap setup. Add steps in `main()` between `load_modules` and `print_status` (or the auto-test block, depending on dependency).

Example — pre-allocate 20 huge pages and mount hugetlbfs:
```sh
mkdir -p /mnt/huge
mount -t hugetlbfs nodev /mnt/huge
echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

After editing `init`, you must rebuild the initrd: `make -C qemu-e2e initrd`.

## Debugging Modes

| Need | Command | Notes |
|---|---|---|
| Interactive shell in VM | `make -C qemu-e2e qemu` | Drops to BusyBox shell after init; `Ctrl-A x` to exit |
| Native-speed run | `make -C qemu-e2e qemu-kvm` | KVM only when host arch == target arch |
| Source-level kernel debug | `make -C qemu-e2e qemu-debug` | Halts at boot waiting for GDB on `:1234` |
| GDB attach | `gdb-multiarch vmlinux -ex 'target remote :1234'` | Run from kernel tree root; needs `vmlinux` (built with `CONFIG_DEBUG_INFO=y`) |
| Block-device tests | `make -C qemu-e2e disk` | Creates `disk.qcow2` (512MB), exposed as `/dev/nvme0n1` in VM |
| PCI passthrough | `QEMU_OPTS='-device vfio-pci,host=XX:XX.X' make -C qemu-e2e qemu` | Host must have IOMMU enabled |

## Triage Playbook

When `qemu-test` fails, walk this list before reporting back to the user:

1. **Harness timeout (exit 124)** — increase `QEMU_TIMEOUT`; if it still hangs, suspect kernel deadlock/livelock or a test infinite loop. Use `make qemu-debug` + GDB and `bt` on the offending CPU.
2. **`Kernel panic` / `Oops` / `BUG:` in serial output** — copy the full stack trace plus the failing instruction; map it to source via `scripts/decode_stacktrace.sh` or `addr2line` against `vmlinux`. Do not "fix" the test until the kernel issue is understood.
3. **Module fails to load (`insmod ...: -1 ...`)** — check ordering in `modules.conf`, missing exported symbols, or a tainted/CONFIG mismatch.
4. **Test binary missing from `/tests/`** — verify the new target is in `CMakeLists.txt` and that `set_target_properties(... RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)` is present; only `build/bin/*` is copied into the initrd.
5. **`Initramfs not found`** — you skipped `make initrd` (or `qemu-test` did, but the build silently failed earlier — re-run `make initrd` standalone and read its output).
6. **`Kernel image not found`** — wrong `ARCH`, or kernel not built; `make verify` catches both.

Never paper over a failure by adding `|| true` or removing assertions. If the user pushes to skip a real failure, push back: regressions caught in `qemu-e2e` are exactly the ones that don't reach mainline review.

## Kernel Coding Style (when editing kernel sources, not the harness)

Follow `Documentation/process/coding-style.rst` in the kernel tree:
- 8-column tabs, 80-column soft limit, no trailing whitespace.
- Only `/* C-style */` comments — never `//`.
- `Signed-off-by:` line in every commit; subject prefixed by subsystem (`mm/hugetlb:`, `zram:`, `crypto: hisi-zip`, etc.).
- Don't introduce GNU C++ idioms; this is C, often `-Wstrict-prototypes`.

The harness itself (`qemu-e2e/`) is user-space C / shell / CMake — normal modern conventions apply there.
