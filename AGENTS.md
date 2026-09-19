# Virtuoso Agent Guide

Virtuoso 是 qemu-e2e（QEMU 内核 E2E 测试装置）的 Rust 化工程。**当前处于 Phase 3
（AI 与全栈现代化，Phase 2 已完成）：Rust workspace 是唯一行为权威**，crate 采用
角色名词命名（Phase 3 重构前的音乐隐喻名：overture→builder、ensemble→launcher、
auditor→judge、coda→guardian、encore→tracker）——
common（基础层）、builder（构建）、launcher（启动 DSL + Firecracker 后端）、
judge（判定 + verdict.json schema）、guardian（进程治理）、tracker（跨 run 聚类/返场）
接管全部实际逻辑；`Makefile` 是转发壳；`infra/*.sh` 保留为行为基线参考
（release 资产流水线仍使用部分脚本）。

## 快速命令（复制即用）

```bash
cargo xtask verify             # 前置检查 + 类型化配置诊断
cargo xtask build              # builder：重建 initrd.img / rootfs.img
cargo xtask test --timeout 60  # 测试：launcher 启动 → judge 判定 → 工件落盘
cargo xtask test --replay-until-fail 5   # flaky 返场：首个非 passed 即停
cargo xtask matrix [--arch a]  # 多架构矩阵（缺省三架构，串行）
cargo xtask triage [--json]    # 最近一次运行的分诊报告
cargo xtask runs [--json]      # 历史运行列表
cargo xtask cluster [--json]   # 跨 run 失败指纹聚类 + flaky 清单 + 首现 run（tracker）
cargo xtask suggest [--diff f] # 补丁↔测试映射：git diff → 最小测试集（tracker）
cargo xtask replay --log <f>   # 任意串口日志的离线标记协议断言
cargo xtask shell [--kvm]      # 交互式 VM
cargo xtask debug              # GDB stub :1234 挂起启动
cargo xtask probe --cmd 'uname -a' [--cmd-file f] [--json]
                               # AI 交互通道：virtio-serial agent 命令批（结构化事件流）
cargo xtask verify --backend firecracker  # microVM preflight（test/shell 同参数）
cargo xtask skill install      # 装 kernel-dev + kernel-virtuoso skill 到内核树
```

AI 的标准验证循环：`verify → test → triage`。**判定以 triage 的 verdict 为准**，
退出码只是接口契约；`verdict: passed` 才算通过。

## Code Map

| 领域 | 入口 | 说明 |
|---|---|---|
| CLI 入口 | `xtask/src/main.rs` | clap 子命令定义 + `cli::dispatch` 分发；Ctrl-C 守护装自 `guardian::registry` |
| CLI 命令组 | `xtask/src/cli/` | `verify.rs`（前置检查）/ `build.rs`（build/busybox/disk/clean/skill）/ `vm.rs`（shell/debug/test/matrix）/ `probe.rs`（AI 交互通道）/ `parity.rs`（make 对照）/ `mod.rs`（分发 + 配置解析 helpers） |
| 类型化配置 | `xtask/src/config.rs` | `.env` 兼容 + `virtuoso.toml` 覆盖层（诊断呈现在 `cli/diagnostics.rs`） |
| 运行工件与分诊 | `xtask/src/runs/` | `rundir.rs`（run 目录、输出泵、verdict.json 落盘/回读）+ `render.rs`（triage/runs/cluster/suggest/replay 呈现） |
| 基础层 | `crates/common/src/` | `arch.rs`（**`Arch` 矩阵唯一事实来源**）/ `fsutil.rs`（which/ELF/可执行位）/ `units.rs`（内存量解析）/ `time.rs` / `fmt.rs`；零依赖 |
| 构建器 | `crates/builder/src/` | busybox 四层供给 / modules.conf / C 用例 / **no_std Rust 用例** / **tools 装载（musl 静态 → /bin）** / cpio+ext4 组装 / verify 检查引擎 |
| 启动 DSL | `crates/launcher/src/` | `qemu.rs`（`QemuInvocation`，argv 与 run-qemu.sh 逐字对齐，QEMU=echo 可对照；`agent_serial` = AI 通道，缺省关）/ `numa.rs` / `lib.rs`（`Backend` 枚举） |
| 第二后端 | `crates/launcher/src/firecracker.rs` | microVM（x86_64/aarch64+KVM）：config JSON / API PUT 序列 / `preflight`+`preflight_checks`；`spawn_supervised` 一体登记监管 |
| 判定引擎 | `crates/judge/src/` | `lib.rs`（标记协议 v1 解析 + `judge` 对账，panic 假通过防护）/ `report.rs`（verdict.json schema + `RunMeta`）/ `exit.rs`（**退出码语义唯一表**） |
| 跨 run 语义 | `crates/tracker/src/lib.rs` | 失败指纹归一化/聚类/flaky/补丁映射/diff→路径；输入是最小摘要 `RunSummary`（`From<VerdictReport>` 投影），IO 在 runs 层 |
| 报告 schema | `crates/judge/src/report.rs::VerdictReport` | verdict.json 唯一 schema（serde 结构体）：构造（`VerdictReport::build`）与回读共用 |
| 进程治理 | `crates/guardian/src/` | `lib.rs`（`ProcessGroupGuard` RAII + `Supervised` 组合句柄）/ `registry.rs`（活动进程组注册表 + Ctrl-C 守护 + 墙钟看门狗） |
| VM 内 init | `infra/init`、`infra/init-initramfs` | PID 1 脚本；`/init-hooks.sh` 为 builder 注入点 |
| C 用例 | `infra/testcases/` | `test_<name>.c` + CMakeLists；`-static` 冻结 |
| Rust 用例 | `infra/testcases/rust/` | 独立 workspace：`testfw` no_std 框架 + 用例 crate；裸 syscall 静态 ELF；`/tests/` 自动发现 |
| VM 内工具 | `infra/tools/` | 独立 workspace（std Rust + **musl 静态**，与 testcases 分类正交）：`agent/` = virtuoso-agent（virtio-serial JSON 行协议，AI probe 的 guest 侧）；装 `/bin/`，不进 `/tests/` 不参与判定 |
| skill | `skills/kernel-dev/`、`skills/kernel-virtuoso/` | `cargo xtask skill install` 装入内核树（后者 = AI 数据接口集成） |

## 运行工件（AI 分诊数据源）

每次 `cargo xtask test` / `matrix` 写入 `target/runs/<unix_ms>-<arch>/`（保留最近 20 次；
`probe` 也写 run 目录，内容为 `serial.log` + `qemu-stderr.log` + `agent-events.jsonl`，无 verdict）：

| 文件 | 内容 |
|---|---|
| `serial.log` | QEMU stdout：内核串口 + 测试标记（含 panic 栈） |
| `qemu-stderr.log` | QEMU 自身告警 |
| `build.log` | 本次构建输出（成功也保留） |
| `events.jsonl` | judge 逐行解析的结构化事件（test_start/test_end/assert/summary/marker/panic/oops/run_end） |
| `verdict.json` | 汇总判定 + 运行指纹（kernel mtime/大小、QEMU 版本、拓扑、超时） |

**Verdict 语义**（`judge::Verdict`）：`passed` / `failed` / `timeout` / `panic` /
`incomplete` / `interrupted` / `build_failed`。注意：`-no-reboot` 下内核 panic 会让
QEMU 以 **exit 0** 退出 —— 只看退出码会假通过；verdict 用 `TEST_COMPLETE` 标记与
退出码对账，panic/oops 独立成档。`exit 0` 但 `verdict: incomplete` = 标记协议没走完，
按失败处理。

## 配置

优先级：`virtuoso.toml` > `.env` > 进程环境变量（未知键解析期报错）。

```toml
# virtuoso.toml 示例
arch = "arm64"
timeout_secs = 60        # 0 一律拒绝
smp = 8                  # 必须被 numa.nodes 整除（解析期校验）
backend = "qemu"         # 或 "firecracker"（microVM：x86_64/aarch64 + KVM）
qemu_opts = ["-device vfio-pci,host=XX:XX.X"]
[numa]
nodes = 2
memory_per_node = "1G"
[busybox]
version = "1.36.1"
# release_repo = "owner/repo"
```

## 冻结的不变量（不要破坏）

1. **标记协议 v1**（`infra/init` 输出，设计文档附录 A）：
   `--- Running: X ---`、`PASSED:/FAILED: X`、`Test Results: N/M passed`、
   `TEST_COMPLETE: ALL TESTS PASSED|SOME TESTS FAILED`。改文本等于破坏所有下游解析。
2. **test 退出码**：0=通过、124=超时、其余=失败。
3. **argv 对齐**：`QemuInvocation::argv` 与 `infra/run-qemu.sh` 逐字对齐
   （验证：`QEMU=echo cargo xtask shell` vs `QEMU=echo infra/run-qemu.sh`）。
4. 测试必须静态链接（`-static`），禁止用 `|| true` 掩盖失败。
5. `.env` 的值优先于进程环境变量（Makefile `-include .env` 遗产语义）。

## 详细文档

- `docs/virtuoso-design.md` —— 总体设计、阶段路线、决策记录（§6 有 Phase 2/3 落地记录）
- `docs/initramfs-rootfs-guide.md` —— 两段式引导逐行解读、"改哪个文件"手册
- `docs/troubleshooting.md` —— Symptom → Solution 速查

## 已知环境怪癖（勿"修复"）

见 `docs/initramfs-rootfs-guide.md` 的怪癖表：空 `/dev` 回退 mknod、sysfs dev 属性
为空、virtio/ext4 必须 `=m` 进 initramfs、BusyBox 裁掉 `CONFIG_TC` 等——都是
openEuler 内核的实测行为，删掉对应 fallback 会重新踩坑。
