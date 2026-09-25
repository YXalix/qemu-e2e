# Troubleshooting — Symptom → Solution

AI/人共用的故障速查。先跑 `virtuoso triage`（或 `virtuoso triage --run <id> --json`），
它直接给出 verdict、测试清单、panic 行与串口尾部；下文的每个症状都假设你已看过对应
`target/runs/<id>/` 下的工件。调试手段（shell / GDB / probe）见
[调试](guide/debugging.md)。

## 超时（exit 124，verdict: timeout）

**Symptom**: `Test timed out after N seconds`，串口停在某处不动。
**Solution**:
1. 看 `serial.log` 尾部：停在哪条打印之后，就是嫌疑点。
2. `verdict.json` 的 `panics` 有内容？→ 转 "panic" 条目（panic 被超时掩盖的形态）。
3. 需要现场调试：`virtuoso shell --gdb` + `gdb-multiarch vmlinux -ex 'target remote :1234'`，
   挂起后发送断点、看 `bt`；或 `virtuoso shell` 手动执行 `/tests/*` 复现。

## panic（verdict: panic）

**Symptom**: `panics` 数组非空，例如 `Kernel panic - not syncing: ...`。
**Solution**:
1. 注意 exit 可能是 0 或 124：`-no-reboot` 下 panic 触发 reset，QEMU 直接退出（0）；
   或卡死到超时（124）。**不要用退出码判断是否发生过 panic。**
2. 解码栈：内核树里 `./scripts/decode_stacktrace.sh vmlinux < serial.log`。
3. panic 在某个测试执行之后？events.jsonl 的最后一条 `test_start` 就是嫌疑人。

## exit 0 但 verdict: incomplete（假通过）

**Symptom**: QEMU 退出码 0，但 `marker_complete` 为 null（没看到 `TEST_COMPLETE`）。
**Solution**: 引导流程没走完就退出了——panic 重置（见上）、init 崩溃、或 rootfs 挂载失败
后 initramfs emergency shell 退出。查 `serial.log` 找 `Kernel panic` 或 `emergency`；
此形态按失败处理，**不得**当作通过。

## verdict: failed（有 FAILED 测试）

**Solution**: `triage` 列出每个测试的断言计数（pass/fail/skip）。找 `[FAIL]` 行上文；
需复现用 `virtuoso shell` 进 VM 手跑 `/tests/test_x`（rootfs 里保留全部用例）。

## insmod 失败 / 模块缺失

**Symptom**: `serial.log` 有 `insmod: can't insert 'xxx.ko'`，或构建期
`Module xxx.ko not found`，或 doctor 阶段 Kernel modules 报 missing。
**Solution**:
- rootfs 的 `modules.conf` 由启用组件的 require 并集生成（`virtuoso.toml`
  `[components.*]`）——加模块改组件的 `require`，不要手写 conf；条目顺序即
  insmod 顺序，被依赖者在前。
- `modules-boot.conf`（initramfs，virtio+ext4 引导关键）仍是手写冻结基础集；
  组件需要引导早期加载时用 `stage = "boot"` 追加。
- 内核重编后 `.ko` 路径变化：`make modules INSTALL_MOD_PATH=...` 或确认
  `virtuoso doctor --verbose` 报告的模块路径。

## 测试二进制没出现在 /tests/

**Symptom**: `Test Results: 0/0 passed` 或根本没有 `--- Running:` 行。
**Solution**: 用例 crate 是否加入了 `infra/testcases/Cargo.toml` 的
`members`；确认静态链接（musl 目标缺省即静态，禁止改成动态）；`virtuoso
clean && virtuoso build` 重建；查 `target/runs/<id>/build.log` 的编译告警。

## kernel image not found / QEMU not found

**Solution**: `virtuoso doctor --verbose` 的前置诊断会给出 MISSING 项与建议
（`KERNEL_PATH`、`ARCH`、`QEMU=` 覆盖）。交叉架构时确认装了对应
`qemu-system-<arch>`。

## SMP not divisible by NUMA_NODES

**Solution**: launcher 校验拒绝启动（SMP 必须整除节点数）。调
`virtuoso.toml` 的 `smp` / `[components.numa]` 的 `nodes`。

## Ctrl-C 中断后想看已输出的日志

**Solution**: 串口是逐行同步落盘的：直接看最新 `target/runs/`（`virtuoso runs`
找最新 id）下的 `serial.log`。verdict.json 缺失时 `triage` 会自动降级为现场解析
serial.log（`verdict: unknown`）。

## BusyBox 下载/构建失败

**Solution**: `virtuoso build --busybox-only` 走四级供应链（本地缓存 → release → 直链 →
源码构建）。离线环境提前把对应架构的 busybox 放进 `target/build/busybox/bin/`；
release 资产校验 ELF magic，损坏会自动回退源码构建。源码兜底仅 Linux 宿主；
applet 符号链接由名单驱动（`infra/busybox/applets-<version>.txt`），自定义
busybox 版本/配置时提供 `BUSYBOX_APPLETS_FILE`。

## macOS 专属

**Symptom**: `Host tools: missing -mke2fs`（doctor/verify ✗）
**Solution**: `brew install e2fsprogs`。keg-only 不进 PATH 没关系，
virtuoso 自动探测 `/opt/homebrew/opt/e2fsprogs/sbin`。

**Symptom**: `Host tools: missing -zig (brew install zig) or CC`
**Solution**: C 测试用例在 macOS 上交叉编译（Apple clang 产不出 Linux 静态
ELF）：`brew install zig`，或自带工具链 `CC=<cross-gcc>` +
`CARGO_TARGET_<TRIPLE>_LINKER=<linker>`。

**Symptom**: `Components: vfio requires Linux host`（verify FAIL）
**Solution**: vfio-pci 直通架构性依赖 Linux IOMMU，macOS 无法支持——注释掉
`[components.vfio]`。pmem 在 macOS 为 experimental（doctor WARN），链路
（memory-backend-file + dumpdtb/fdtput）未经 HVF 实测前不要依赖。

**Symptom**: macOS 上跑 x86_64 / riscv64 guest 极慢或超时
**Solution**: 交叉 guest 只能 TCG（HVF 仅同构 arm64）。放大
`timeout_secs`，或回到 Linux 宿主跑全矩阵（`virtuoso matrix`）。

**Symptom**: `kernel.sh` 报 git clone 后构建大小写冲突 / 内核树行为诡异
**Solution**: 内核源码必须在大小写敏感文件系统——`kernel.sh` 的 named volume
（ext4）正确；自行 bind-mount APFS 目录会踩坑，别这么做。

## 工件占满磁盘

**Solution**: 每次运行保留最近 20 次（`xtask/src/runs/rundir.rs` 的 `RUNS_KEEP`），
`target/runs/` 在 `/target` 下随 `cargo clean` 一并清除；单次工件通常 < 1 MiB。
