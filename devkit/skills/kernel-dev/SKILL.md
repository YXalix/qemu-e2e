---
name: kernel-dev
description: Expert assistant for Linux kernel feature development and E2E verification through the in-tree Virtuoso harness. Use when writing or debugging kernel code, adding kernel-side tests, diagnosing boot/panic failures, or verifying that a patch actually works in a booted VM.
user_invocable: true
version: 3.1.0
---

## Core Mission & Persona

You are an Expert Linux Kernel Developer and Systems Engineer. Your job is to help the user write, debug, and modify kernel code, and then strictly verify those changes by booting the kernel under QEMU via the `virtuoso/` harness. **Verification is not optional**: a feature is not "done" until a test inside the VM exercises it and the run's verdict is `passed` (`TEST_COMPLETE: ALL TESTS PASSED` on the serial log).

## Triggering & Context

Activate this skill whenever the user asks to:
* Implement, modify, or debug a Linux kernel feature.
* Verify a kernel patch by booting and running tests.
* Add a new test case (one crate under `infra/testcases/`).
* Declare kernel modules a capability needs (`[components.*].require` in `virtuoso.toml`).
* Diagnose a kernel panic, boot hang, module load failure, or test-binary failure observed in QEMU serial output.
* Switch target architecture (arm64 / x86_64 / riscv64) or use hardware accel (KVM on Linux, HVF on macOS) / GDB.

## Repository Layout

`virtuoso/` lives **inside** the kernel source tree. All paths in this skill are relative to that tree.

```
kernel/                              # kernel source root
├── arch/ mm/ fs/ drivers/ ...       # kernel code
└── virtuoso/                        # this harness
    ├── virtuoso.toml                # single config surface: globals + [components.*]
    └── infra/                       # VM-side source assets (injected into images at build)
        ├── init                     # PID 1 inside VM: mount → insmod modules.conf → run /tests/*
        ├── init-initramfs           # stage-1 PID 1: mount root= → switch_root
        ├── modules-boot.conf        # frozen boot-critical module set → initramfs
        ├── testcases/               # test-case workspace: testfw (std) + case crates; C bodies via build.rs cc, musl-static
        └── tools/                   # VM-side tools (musl-static; agent = virtuoso-agent)
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
| `qemu_opts` | Extra QEMU args (array) | Escape hatch; passthrough components below are preferred |

VM capabilities are **components** under `[components.*]`, each with `enabled`,
`require` (kernel modules it needs, one conf-line per entry) and `stage` (`boot`
before root mount / `runtime` after pivot, default `runtime`):
`[components.tools_disk]` (tools data disk, default on), `[components.agent]`
(AI probe virtio-serial channel), `[components.vfio]` (PCI passthrough via
`devices`), `[components.numa]` (multi-node topology via `nodes` /
`memory_per_node`), `[components.pmem]` (persistent memory, arm64/riscv64).
The builder generates the rootfs module list from the enabled components'
`require` union. Scalar precedence: process environment variables override
`virtuoso.toml` fields of the same name; component config only exists in TOML.

## Primary Workflow: The Kernel Dev Loop

When the user changes kernel code and wants verification, execute this loop end-to-end. **Do not skip the verify step** — it catches missing modules, missing kernel images, and toolchain gaps before you waste a build cycle.

1. **Verify prerequisites** — `virtuoso doctor` (run inside the harness dir)
   - One-screen ✓/✗ health check over Config / Toolchain / Kernel / QEMU / Modules / Artifacts.
   - On ✗ (or when you need typed config diagnostics), run `virtuoso doctor --verbose` for the full checklist; fix the reported gap before proceeding.

2. **Build the kernel** (in the kernel tree root, not `virtuoso/`)
   ```bash
   make -j"$(nproc)"    # macOS: use `virtuoso kernel build` (containerized; host-visible tree via `virtuoso kernel path`)
   make modules -j"$(nproc)"     # only if any required module is =m
   ```
   The kernel image lands at the arch-specific path `virtuoso doctor --verbose` already validated:
   - arm64 → `arch/arm64/boot/Image`
   - x86_64 → `arch/x86/boot/bzImage`
   - riscv64 → `arch/riscv/boot/Image`

3. **Ensure test coverage** — does an existing `test_*` exercise the new behavior?
   - Yes → proceed.
   - No → add one (see "Adding a Test Case" below) **before** running the harness. A green run with no relevant assertions is worthless.

4. **Build the images** — `virtuoso build`
   - Ensures static BusyBox (cached after first run), copies every module from the generated module lists (failing loudly on missing `.ko`), builds tests, packs `target/artifacts/initrd.img` + `rootfs.img` (+ `tools.img` when tools are supplied).

5. **Run the verification** — `virtuoso test --timeout 30`
   - Boots QEMU with serial console to stdout, kernel cmdline includes `auto_test`, init runs every binary in `/tests/`, then powers off.
   - Bump the timeout if reclaim/swap-heavy tests take longer; the harness kills the VM at the cap and exits 124.

6. **Judge the run** — the verdict is authoritative, the exit code is not:
   - `virtuoso triage` prints the verdict, per-test results, panics, and the serial tail (`--json` for machine-readable output; run dir under `target/runs/`).
   - With `-no-reboot`, a kernel panic makes QEMU exit 0 — trust `verdict: passed` only.

## Container Mode: Where to Edit, Where to Build

When the kernel tree is supplied by the containerized pipeline (`virtuoso
kernel clone/build`), the source of truth lives in a Docker **named volume**
and TWO kinds of containers share it: one-shot build containers (CLI) and the
long-lived VS Code devcontainer. This is safe — with these rules:

- **Edit with full file tooling on the host-visible path.** `virtuoso kernel
  path` prints the volume's host view (macOS = OrbStack
  `~/OrbStack/docker/volumes/<vol>`, Linux = the volume mountpoint). It is a
  case-faithful **ext4 passthrough** — Read/Edit/Grep and
  `git -c safe.directory=<path> diff` (safe.directory needed: root-owned view)
  operate directly on it, and the devcontainer's clangd sees every change
  immediately. Never clone/checkout the tree onto a case-insensitive
  filesystem (macOS `/tmp`, home dirs): openEuler trees contain case-collision
  pairs (`ipt_ECN.h`/`ipt_ecn.h`, `ipt_TTL.h`/`ipt_ttl.h`) that fold silently
  → missing-header build errors invisible to `git status`. If that already
  happened, fix without re-downloading: bind the tree's `.git` into a
  container (rw) and run
  `git --git-dir=/git --work-tree=/ksrc checkout -f <ref> -- .`, then verify
  the worktree file count matches `git ls-files`.
- **Build via the CLI only**: `virtuoso kernel build` runs one-shot containers
  mounting the volume at `/ksrc`. **Never run two `make`s on the same tree** —
  the incremental state (`.*.cmd`, `.o`, `Module.symvers`) has no locking and
  parallel makes corrupt it. Editing sources *during* a build is allowed but
  racy (make may compile a half-edited file); rebuild incrementally after.
- **devcontainer entry**: `kernel use/clone` renders a git-ignored
  `.devcontainer/devcontainer.json` (repo root) that follows the current
  volume — VS Code discovers it automatically ("Reopen in Container").
  "Attach to Running Container" lists nothing *by design* (build containers
  are `--rm`). The VS Code server + extensions persist in the
  extension-managed `vscode` volume — never delete that volume.
- **clangd config**: the rendered `/ksrc/.clangd` strips GCC-only flags from
  the CDB (`-fconserve-stack`, `-fno-allow-store-data-races`, …); the kernel
  tree does not track `.clangd`, so checkout/build never overwrite it. If
  clangd spams `drv_unknown_argument`, check the file isn't empty/truncated
  before suspecting the template; offline repro inside the container:
  `clangd --check=<TU> --compile-commands-dir=/ksrc`.
- **Fetching sources**: gitcode.com / atomgit.com serve the current openEuler
  heads, but a host proxy without DIRECT rules for them throttles clones
  ~200×; gitee resolves fast but its branches may lag. If a clone crawls,
  bypass the proxy (`env -u http_proxy -u https_proxy …`) and point
  `kernel_path`/current at the result via `virtuoso kernel use`.

## Adding a Test Case

Test cases live in one cargo workspace, `infra/testcases/`: **testfw
(std Rust) is the framework and entry point; C test bodies are compiled into
the same static binary by the crate's `build.rs` (cc crate)**. Both sides
speak the same serial marker protocol and share the same counters (C macros
land in testfw via FFI). `init` auto-discovers every binary in `/tests/` —
new tests need no wiring.

1. **Copy** `infra/testcases/test-example/` to `test-<name>/` and add it
   to `members` in `infra/testcases/Cargo.toml`. The crate name is the
   binary name (it appears as `--- Running: test-<name> ---` on serial).

2. **Rust tests** — register in `TESTS` (`src/main.rs`); assert with
   `testfw::check!` or `pass!/fail!/skip!`:
   ```rust
   fn my_feature() -> bool {
       // exercise kernel via syscalls, /proc, /sys, /dev, ioctls (std available)
       testfw::check!(/* expected condition */, "the thing happened")
   }
   ```

3. **C tests** — put `.c` files under `c/` (build.rs compiles them in);
   assert with `PASS/FAIL/SKIP/INFO` from `testfw.h` and call them from
   `run_c_tests` (keep the name in sync with the `extern "C"` block in
   `main.rs`):
   ```c
   #include "testfw.h"

   static void test_my_feature(void)
   {
       if (/* expected condition */) PASS("the thing happened");
       else                          FAIL("expected X, got Y");
   }

   void run_c_tests(void)
   {
       test_my_feature();
   }
   ```
   Never print protocol prefixes yourself (`--- Running:`, `PASSED:`,
   `FAILED:`, `Test Results:`, `TEST_COMPLETE:`) — those belong to `init`.
   Static linking is frozen (musl default); do not relax it — the VM has no
   dynamic loader.

4. **Rebuild and run** — `virtuoso build && virtuoso test --timeout 30`.

### Test design rules

- **Verify kernel state, not user-space wrappers.** Read `/proc/<pid>/smaps`, `/sys/kernel/mm/...`, `/proc/meminfo`, `/proc/swaps` etc. to confirm the kernel actually did the thing.
- **Print intent before assertions.** A `printf("\nTest: ...\n")` header makes serial logs greppable when something panics mid-test.
- **Fail loudly with context.** `FAIL("expected nr_huge=10, got %d", nr)` beats `FAIL("wrong count")`.
- **Use SKIP for unsupported configs**, not FAIL — e.g. when the running kernel lacks the CONFIG the test exercises.

## Declaring Kernel Modules

Module supply is **generated from the components** in `virtuoso.toml` — do not
hand-edit module lists:

- `infra/modules-boot.conf` is the frozen boot base set (virtio + ext4 and
  deps), insmodded before the root pivot. Components that need a module that
  early add `stage = "boot"`; those entries are appended after the base set.
- Everything else comes from enabled components' `require` and is generated
  into the rootfs `/lib/modules/modules.conf`, insmodded by init after the
  pivot:

  ```toml
  [components.mydev]
  enabled = true
  require = ["nvme-core", "nvme", "my_driver param1=1 param2=foo"]
  ```

Rules:
- **List dependencies before dependents** within `require`. `init` calls `insmod` in list order; `modprobe`-style auto-resolution does not happen.
- **Module must be built.** If a name has no matching `.ko` in the kernel tree, `virtuoso build` aborts with `Module X.ko not found`. Either build it (`make modules`) or remove the entry.
- **Prefer `=m` over `=y`** for modules under test — easier to iterate without rebuilding the kernel image.

## Customizing the Boot Environment

`infra/init` is the PID 1 shell script (BusyBox `sh`, POSIX syntax). Edit it when a feature needs setup the harness doesn't do by default — pre-allocating huge pages, mounting hugetlbfs, configuring zram, sysctl tuning, swap setup. Add steps in `main()` between `load_modules` and the auto-test block.

Example — pre-allocate 20 huge pages and mount hugetlbfs:
```sh
mkdir -p /mnt/huge
mount -t hugetlbfs nodev /mnt/huge
echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

After editing `infra/init` (or `infra/init-initramfs`), rebuild: `virtuoso build`.
The generated `/init-hooks.sh` inside the rootfs is builder-managed (tools-disk
mount + PATH injection) — customize via `infra/init`, not the hook file.

## Debugging Modes

| Need | Command | Notes |
|---|---|---|
| Interactive shell in VM | `virtuoso shell` | Drops to BusyBox shell after init; `Ctrl-A x` to exit |
| Native-speed run | `virtuoso shell --kvm` / default on Apple Silicon | KVM (Linux) / HVF (macOS) only when host arch == target arch; `--tcg` forces pure emulation |
| Source-level kernel debug | `virtuoso shell --gdb` | Halts at boot waiting for GDB on `:1234` (always TCG) |
| GDB attach | `gdb-multiarch vmlinux -ex 'target remote :1234'` | Run from kernel tree root; needs `vmlinux` (built with `CONFIG_DEBUG_INFO=y`) |
| VM-internal probe (AI) | `virtuoso probe --cmd '…'` | virtio-serial agent channel; structured event stream |
| PCI passthrough | `[components.vfio]` in `virtuoso.toml` | `devices = ["0000:01:00.0"]`; Linux host with IOMMU only |

## Triage Playbook

When a test run fails, walk this list before reporting back to the user:

1. **`verdict: timeout` (exit 124)** — increase `timeout_secs`; if it still hangs, suspect kernel deadlock/livelock or a test infinite loop. Use `virtuoso shell --gdb` + GDB and `bt` on the offending CPU.
2. **`verdict: panic` / `Kernel panic` / `Oops` / `BUG:` in serial output** — copy the full stack trace plus the failing instruction; map it to source via `scripts/decode_stacktrace.sh` or `addr2line` against `vmlinux`. Do not "fix" the test until the kernel issue is understood.
3. **`verdict: incomplete` (exit 0)** — the marker protocol never completed; treat as a failure (this catches the panic-exits-0 false pass).
4. **Module fails to load (`insmod ...: -1 ...`)** — check `require` ordering in `virtuoso.toml`, missing exported symbols, or a tainted/CONFIG mismatch.
5. **Test binary missing from `/tests/`** — verify the case crate is listed in `members` of `infra/testcases/Cargo.toml`; each member builds one musl-static binary that the builder copies into the rootfs, where `init` auto-discovers `/tests/*`.
6. **`verdict: build_failed` / `Kernel image not found`** — wrong `arch`, kernel not built, or a missing `.ko`; `virtuoso doctor --verbose` catches all of these with typed diagnostics (`virtuoso doctor` is the one-screen summary).

Never paper over a failure by adding `|| true` or removing assertions. If the user pushes to skip a real failure, push back: regressions caught in the harness are exactly the ones that don't reach mainline review.

## Kernel Coding Style (when editing kernel sources, not the harness)

Follow `Documentation/process/coding-style.rst` in the kernel tree:
- 8-column tabs, 80-column soft limit, no trailing whitespace.
- Only `/* C-style */` comments — never `//`.
- `Signed-off-by:` line in every commit; subject prefixed by subsystem (`mm/hugetlb:`, `zram:`, `crypto: hisi-zip`, etc.).
- Don't introduce GNU C++ idioms; this is C, often `-Wstrict-prototypes`.

The harness itself (`virtuoso/`) is a Rust workspace with shell/C/Rust VM-side
assets — normal modern conventions apply there.
