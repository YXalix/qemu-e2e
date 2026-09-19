# Virtuoso Agent Guide

Virtuoso 是 qemu-e2e（QEMU 内核 E2E 测试装置）的 Rust 化工程。**当前处于 Phase 3
（AI 与全栈现代化，Phase 2 已完成）：Rust workspace 是唯一行为权威**，crate 采用
角色名词命名（Phase 3 重构前的音乐隐喻名：overture→builder、ensemble→launcher、
auditor→judge、coda→guardian、encore→tracker）——
common（基础层）、builder（构建）、launcher（启动 DSL + Firecracker 后端）、
judge（判定 + verdict.json schema）、guardian（进程治理）、tracker（跨 run 聚类/返场）
接管全部实际逻辑；`Makefile` 是转发壳。原 shell 基线脚本（run-qemu/verify/
build-initrd/cpio2ext4/fetch-busybox/init-rootfs）已全部被 Rust 接管并于 2026-09
移除；`infra/` 只保留 VM 内源资产（init、init-initramfs、modules-boot.conf（冻结
基础集）、testcases/、tools/），git 跟踪（rootfs 的 modules.conf 由组件 require
并集生成，不再手写）；构建产物与缓存一律落 `target/`（数据面，git 忽略）：
`target/artifacts/`（initrd.img / rootfs.img / tools.img）+ `target/build/`
（busybox 供给缓存、initramfs/rootfs/tools 暂存目录）。

## 快速命令（复制即用）

```bash
cargo xtask verify             # 前置检查 + 类型化配置诊断
cargo xtask build              # builder：重建 initrd.img / rootfs.img / tools.img
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
| CLI 命令组 | `xtask/src/cli/` | `verify.rs`（前置检查）/ `build.rs`（build/busybox/clean/skill）/ `vm.rs`（shell/debug/test/matrix）/ `probe.rs`（AI 交互通道）/ `parity.rs`（make 对照）/ `mod.rs`（分发 + 配置解析 helpers，含 `tools_disk_opt`） |
| 类型化配置 | `xtask/src/config.rs` | **`virtuoso.toml` 唯一配置面**：全局键 + `[components.*]` 组件化（`require` = KO 依赖，`ComponentPlan` 并集分区 boot/runtime）；`.env` 已废弃（存在 WARN 仍读，诊断呈现在 `cli/diagnostics.rs`） |
| 运行工件与分诊 | `xtask/src/runs/` | `rundir.rs`（run 目录、输出泵、verdict.json 落盘/回读）+ `render.rs`（triage/runs/cluster/suggest/replay 呈现） |
| 基础层 | `crates/common/src/` | `arch.rs`（**`Arch` 矩阵唯一事实来源**）/ `fsutil.rs`（which/ELF/可执行位）/ `units.rs`（内存量解析）/ `time.rs` / `fmt.rs`；零依赖 |
| 构建器 | `crates/builder/src/` | busybox 四层供给 / **模块清单生成（modconf：boot 基础集 + 组件 require 并集）** / C 用例 / **no_std Rust 用例** / **tools 装载（musl 静态 → tools.img，外挂数据盘）** / cpio+ext4 组装 / verify 检查引擎 |
| 启动 DSL | `crates/launcher/src/` | `qemu.rs`（`QemuInvocation`，argv 由单测冻结在历史 shell 基线；`QEMU=echo` 可打印对照；`data_disks` = 多 virtio-blk 数据盘，缺省空；`agent_serial` = AI 通道，缺省关）/ `numa.rs` / `lib.rs`（`Backend` 枚举 + `DataDisk`） |
| 第二后端 | `crates/launcher/src/firecracker.rs` | microVM（x86_64/aarch64+KVM）：config JSON / API PUT 序列 / 多 drive（`data_disks`）/ `preflight`+`preflight_checks`；`spawn_supervised` 一体登记监管 |
| 判定引擎 | `crates/judge/src/` | `lib.rs`（标记协议 v1 解析 + `judge` 对账，panic 假通过防护）/ `report.rs`（verdict.json schema + `RunMeta`）/ `exit.rs`（**退出码语义唯一表**） |
| 跨 run 语义 | `crates/tracker/src/lib.rs` | 失败指纹归一化/聚类/flaky/补丁映射/diff→路径；输入是最小摘要 `RunSummary`（`From<VerdictReport>` 投影），IO 在 runs 层 |
| 报告 schema | `crates/judge/src/report.rs::VerdictReport` | verdict.json 唯一 schema（serde 结构体）：构造（`VerdictReport::build`）与回读共用 |
| 进程治理 | `crates/guardian/src/` | `lib.rs`（`ProcessGroupGuard` RAII + `Supervised` 组合句柄）/ `registry.rs`（活动进程组注册表 + Ctrl-C 守护 + 墙钟看门狗） |
| VM 内 init | `infra/init`、`infra/init-initramfs` | PID 1 脚本；`/init-hooks.sh` 为 builder 注入点 |
| C 用例 | `infra/testcases/` | `test_<name>.c` + CMakeLists；`-static` 冻结 |
| Rust 用例 | `infra/testcases/rust/` | 独立 workspace：`testfw` no_std 框架 + 用例 crate；裸 syscall 静态 ELF；`/tests/` 自动发现 |
| VM 内工具 | `infra/tools/` | 独立 workspace（std Rust + **musl 静态**，与 testcases 分类正交）：`agent/` = virtuoso-agent（virtio-serial JSON 行协议，AI probe 的 guest 侧）；装 tools.img 的 `/bin/`（VM 内挂 `/tools`，init-hooks 注入 PATH），不进 `/tests/` 不参与判定 |
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

## 配置（组件化）

`virtuoso.toml` 是唯一配置面；优先级：`virtuoso.toml` > `.env`（废弃，存在 WARN）>
进程环境变量；未知键/非法类型解析期报错。**未配置项以注释形式存在于仓库根的
`virtuoso.toml` 模板**（活动行 = 默认常规启动配置）。VM 能力按组件配置，每个
`[components.*]` 段可用 `enabled` / `require`（KO 依赖，条目 = conf 行
`"<module> [key=val ...]"`）/ `stage`（`boot`｜`runtime`，缺省 runtime）：

```toml
# virtuoso.toml 示例（完整注释模板见仓库根）
arch = "arm64"           # x86_64 | arm64 | riscv64
timeout_secs = 60        # 0 一律拒绝
smp = 8                  # 多节点 NUMA 时须被 nodes 整除（解析期校验）
backend = "qemu"         # 或 "firecracker"（microVM：x86_64/aarch64 + KVM）
auto_test = true
qemu_opts = ["-device ivshmem-plain,memdev=hostmem"]   # 透传兜底

[components.tools_disk]  # tools.img → /dev/vdb 挂 /tools；段缺省 = 启用
enabled = true
# [components.agent]     # AI probe 通道（virtio-serial）；段缺省 = 关闭
# enabled = true
# require = ["virtio_console"]
# [components.vfio]      # 直通：devices 逐条生成 -device vfio-pci,host=<bdf>
# enabled = true
# devices = ["0000:01:00.0"]
# [components.numa]      # 多节点拓扑；缺省单节点
# enabled = true
# nodes = 2
# memory_per_node = "1G"
# [components.pmem]      # 持久内存（DT 途径 → /dev/pmem0 + DAX）；段缺省 = 关闭
# enabled = true
# size = "256M"          # 从 guest RAM 顶部挖出；cmdline 加 mem= 排除；主内存后端换宿主文件（持久）
# require = ["libnvdimm", "nd_btt", "of_pmem", "nd_pmem"]   # 内核 =m 时声明（含顺序）
# [busybox]
# version = "1.36.1"
```

builder 把启用组件的 require 并集（schema 固定顺序
tools_disk→agent→vfio→numa→pmem，去重保首个）生成 rootfs
`/lib/modules/modules.conf`；`stage = "boot"` 的条目追加到
initramfs 的 modules-boot.conf 冻结基础集之后。`cargo xtask probe` 恒开 agent 通道
（强制并入 virtio_console，不依赖组件开关）。firecracker 后端不支持 agent 通道与
pmem 组件（启用时 WARN 忽略）。

## 冻结的不变量（不要破坏）

1. **标记协议 v1**（`infra/init` 输出，设计文档附录 A）：
   `--- Running: X ---`、`PASSED:/FAILED: X`、`Test Results: N/M passed`、
   `TEST_COMPLETE: ALL TESTS PASSED|SOME TESTS FAILED`。改文本等于破坏所有下游解析。
2. **test 退出码**：0=通过、124=超时、其余=失败。
3. **argv 冻结**：`QemuInvocation::argv` 的输出冻结在原 `infra/run-qemu.sh`
   （已删除）的基线上，由 `crates/launcher/src/qemu.rs` 的 `argv_*` 单测把守；
   人工复核用 `QEMU=echo cargo xtask shell` 打印 argv。数据盘（tools.img 等，
   追加 `-drive …,if=virtio` → `/dev/vdb` 起）与 agent 通道属调用方增量：
   **缺省（无盘无 agent）argv 与基线逐字一致**。NVMe 测试盘（disk.qcow2）已于
   2026-09 移除，块设备抽象统一为 `DataDisk`（QEMU/Firecracker 双后端多盘）。
4. 测试必须静态链接（`-static`），禁止用 `|| true` 掩盖失败。
5. `.env` 的值优先于进程环境变量（Makefile `-include .env` 遗产语义）——但
   `.env` 已废弃：仅在文件存在时打 WARN 兼容读取（仅标量键），组件化配置只能
   写 `virtuoso.toml`；优先级 toml > .env > 进程 env。

## 详细文档

- `docs/virtuoso-design.md` —— 总体设计、阶段路线、决策记录（§6 有 Phase 2/3 落地记录）
- `docs/initramfs-rootfs-guide.md` —— 两段式引导逐行解读、"改哪个文件"手册
- `docs/troubleshooting.md` —— Symptom → Solution 速查

## 已知环境怪癖（勿"修复"）

见 `docs/initramfs-rootfs-guide.md` 的怪癖表：空 `/dev` 回退 mknod、sysfs dev 属性
为空、virtio/ext4 必须 `=m` 进 initramfs、BusyBox 裁掉 `CONFIG_TC` 等——都是
openEuler 内核的实测行为，删掉对应 fallback 会重新踩坑。
