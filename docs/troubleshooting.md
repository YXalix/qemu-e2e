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
3. 需要现场调试：`virtuoso debug` + `gdb-multiarch vmlinux -ex 'target remote :1234'`，
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
`Module xxx.ko not found`，或 verify 阶段 Kernel modules 报 missing。
**Solution**:
- rootfs 的 `modules.conf` 由启用组件的 require 并集生成（`virtuoso.toml`
  `[components.*]`）——加模块改组件的 `require`，不要手写 conf；条目顺序即
  insmod 顺序，被依赖者在前。
- `modules-boot.conf`（initramfs，virtio+ext4 引导关键）仍是手写冻结基础集；
  组件需要引导早期加载时用 `stage = "boot"` 追加。
- 内核重编后 `.ko` 路径变化：`make modules INSTALL_MOD_PATH=...` 或确认
  `virtuoso verify` 报告的模块路径。

## 测试二进制没出现在 /tests/

**Symptom**: `Test Results: 0/0 passed` 或根本没有 `--- Running:` 行。
**Solution**: `infra/testcases/CMakeLists.txt` 是否注册了新用例；确认 `-static`
（禁止改成动态链接）；`virtuoso clean && virtuoso build` 重建；查
`target/runs/<id>/build.log` 的编译告警。

## kernel image not found / QEMU not found

**Solution**: `virtuoso verify` 的前置诊断会给出 MISSING 项与建议
（`KERNEL_PATH`、`ARCH`、`QEMU=` 覆盖）。交叉架构时确认装了对应
`qemu-system-<arch>`。

## SMP not divisible by NUMA_NODES

**Solution**: launcher 校验拒绝启动（SMP 必须整除节点数）。调
`virtuoso.toml` 的 `smp` / `[components.numa]` 的 `nodes`。

## Ctrl-C 中断后想看已输出的日志

**Solution**: 串口是逐行同步落盘的：直接看最新 `target/runs/`（`virtuoso runs`
找最新 id）下的 `serial.log`。verdict.json 缺失时 `triage` 会自动降级为现场解析
serial.log（`verdict: unknown`）。

## make qemu-test 与 virtuoso 退出码不一致

**Symptom**: parity 报 CI-equivalent 或 FAIL。
**Solution**: 已知差异——GNU make 把脚本失败折叠为 2、吞掉 `exit 124`；virtuoso 保留
真实码（退出码语义见[冻结契约](architecture/contracts.md)）。`virtuoso parity <target> --strict`
可强制严格比较。以 virtuoso 为权威。

## BusyBox 下载/构建失败

**Solution**: `virtuoso busybox` 走四级供应链（本地缓存 → release → 直链 →
源码构建）。离线环境提前把对应架构的 busybox 放进 `target/build/busybox/bin/`；
release 资产校验 ELF magic，损坏会自动回退源码构建。

## 工件占满磁盘

**Solution**: 每次运行保留最近 20 次（`xtask/src/runs/rundir.rs` 的 `RUNS_KEEP`），
`target/runs/` 在 `/target` 下随 `cargo clean` 一并清除；单次工件通常 < 1 MiB。
