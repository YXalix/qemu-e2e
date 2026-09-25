# Virtuoso Agent Guide

Virtuoso 是 QEMU 内核 E2E 测试装置。**Rust workspace 是唯一行为权威**，crate 以
角色名词命名——
common（基础层）、builder（构建）、launcher（启动 DSL）、
judge（判定 + verdict.json schema）、guardian（进程治理）、tracker（跨 run 聚类/返场）
接管全部实际逻辑；`infra/` 只保留源资产（init、
init-initramfs、modules-boot.conf（冻结基础集）、testcases/、tools/、
busybox/applets-*.txt（applet 名单冻结数据）、kernel/（preset 内核
fragment/pin/构建脚本）），git 跟踪
（rootfs 的 modules.conf 由组件 require 并集生成）；`devkit/` 收纳内核开发外围工具
（`devkit/docker/` 容器化内核开发环境 = macOS 内核供给 + clangd、`devkit/skills/`
AI skill 源，见 Code Map）；构建产物与缓存一律落
`target/`（数据面，git 忽略）：`target/artifacts/`（initrd.img / rootfs.img /
tools.img）+ `target/build/`（busybox 供给缓存、initramfs/rootfs/tools 暂存目录）
+ `target/kernel/preset/`（fetch 下来的预编内核缓存）。

## 快速命令（复制即用）

`virtuoso` = 规范二进制（`cargo install --path cli` 装入 PATH）；改完 CLI 源码
重跑一次安装即可保持最新。

```bash
virtuoso doctor             # 环境体检（✓/✗ 组件行；--verbose 全量诊断；--json）
virtuoso build              # builder：重建 initrd.img / rootfs.img / tools.img（--busybox-only 仅备 BusyBox）
virtuoso fetch [--version v] [--arch a]
                            # 拉 preset 预编内核（mainline mini Image）→ target/kernel/preset
virtuoso test --timeout 60  # 测试：launcher 启动 → judge 判定 → 工件落盘
virtuoso test --replay-until-fail 5   # flaky 返场：首个非 passed 即停
virtuoso matrix [--arch a]  # 多架构矩阵（缺省三架构，串行）
virtuoso triage [--json]    # 最近一次运行的分诊报告
virtuoso runs [--json]      # 历史运行列表
virtuoso cluster [--json]   # 跨 run 失败指纹聚类 + flaky 清单 + 首现 run（tracker）
virtuoso suggest [--diff f] # 补丁↔测试映射：git diff → 最小测试集（tracker）
virtuoso replay --log <f>   # 任意串口日志的离线标记协议断言
virtuoso shell [--kvm|--tcg] [--gdb]
                            # 交互式 VM；--gdb = 挂起等 GDB :1234（恒 TCG）
virtuoso probe --cmd 'uname -a' [--cmd-file f] [--json]
                               # AI 交互通道：virtio-serial agent 命令批（结构化事件流）
virtuoso skill install      # 装 kernel-dev + kernel-virtuoso skill 到内核树
virtuoso docs [--serve]     # mdBook 文档构建到 target/book / 本地预览
```

AI 的标准验证循环：`doctor → test → triage`。**判定以 triage 的 verdict 为准**，
退出码只是接口契约；`verdict: passed` 才算通过。doctor 是体检唯一入口（一屏
呈现 + `--verbose` 全量），与 builder::verify 检查引擎同源（新增前置条件只动
引擎，呈现自动跟随）。

## Code Map

| 领域 | 入口 | 说明 |
|---|---|---|
| CLI 入口 | `cli/src/main.rs` | 包名 = bin 名 = `virtuoso`：clap 子命令定义 + `cli::dispatch` 分发；Ctrl-C 守护装自 `guardian::registry` |
| CLI 命令组 | `cli/src/cli/` | `doctor.rs`（体检唯一入口：一屏分组呈现 + `--verbose` 全量；`engine_report` 引擎投影同文件）/ `build.rs`（build/clean/skill；`--busybox-only` 仅备 BusyBox）/ `vm.rs`（shell/test/matrix；accel 解析：macOS 同构缺省 HVF、`--tcg` 强制、Linux 恒 TCG；shell `--gdb` 挂起等 GDB）/ `probe.rs`（AI 交互通道）/ `mod.rs`（分发 + 配置解析 helpers，含 `tools_disk_opt`） |
| 类型化配置 | `cli/src/config.rs` | **`virtuoso.toml` 唯一配置面**：全局键 + `[components.*]` 组件化（`require` = KO 依赖，`ComponentPlan` 并集分区 boot/runtime）；标量键优先级 进程环境变量 > toml，诊断呈现在 `cli/diagnostics.rs` |
| 运行工件与分诊 | `cli/src/runs/` | `rundir.rs`（run 目录、输出泵、verdict.json 落盘/回读）+ `render.rs`（triage/runs/cluster/suggest/replay 呈现） |
| 基础层 | `crates/common/src/` | `arch.rs`（**`Arch` 矩阵唯一事实来源**）/ `platform.rs`（**`HostOs` = QEMU 平台分支唯一事实来源**）/ `fsutil.rs`（which/ELF/可执行位）/ `units.rs`（内存量解析）/ `time.rs` / `fmt.rs`；零依赖 |
| 构建器 | `crates/builder/src/` | busybox 四层供给（applet 符号链接由 `infra/busybox/applets-<ver>.txt` 名单驱动，不执行 guest ELF）/ **模块清单生成（modconf：boot 基础集 + 组件 require 并集）** / **用例编译（C over Rust：testfw std 框架 + 用例 crate 的 build.rs cc 编 C，恒 `--target <musl triple>`）** / **tools 装载（musl 静态 → tools.img，外挂数据盘）** / `cpio.rs`（**initrd 原生 newc+gzip**，无 GNU 工具依赖、确定性输出）/ `preset.rs`（**kernel_preset 预编内核供给**：pin/release 版本解析 + gh/直链下载 + SHA256 校验；供给端 = kernel-release.yml）/ `cross.rs`（**全宿主交叉接线**：zig cc 包装（`-target` 追加式覆盖外部 rust 风格 target）→ `CC_<TRIPLE>`（C 测试体，全宿主）/ `CARGO_TARGET_*_LINKER`（仅非 Linux 宿主））/ `image.rs`（mke2fs 探测含 brew keg 路径） / verify 检查引擎（host_tools 按平台分表，zig 替位 cmake；preset 激活时源码树/.ko 检查降级为 preset 语义） |
| 启动 DSL | `crates/launcher/src/` | `qemu.rs`（`QemuInvocation`，argv 冻结在**按宿主平台的双基线**：Linux=memfd+KVM、macOS=ram+HVF，accel×平台错配在 argv 构造期报错；`QEMU=echo` 可打印对照；`data_disks` = 多 virtio-blk 数据盘，缺省空；`agent_serial` = AI 通道，缺省关）/ `numa.rs` / `lib.rs`（`DataDisk` 块设备抽象） |
| 判定引擎 | `crates/judge/src/` | `lib.rs`（标记协议 v1 解析 + `judge` 对账，panic 假通过防护）/ `report.rs`（verdict.json schema + `RunMeta`）/ `exit.rs`（**退出码语义唯一表**） |
| 跨 run 语义 | `crates/tracker/src/lib.rs` | 失败指纹归一化/聚类/flaky/补丁映射/diff→路径；输入是最小摘要 `RunSummary`（`From<VerdictReport>` 投影），IO 在 runs 层 |
| 报告 schema | `crates/judge/src/report.rs::VerdictReport` | verdict.json 唯一 schema（serde 结构体）：构造（`VerdictReport::build`）与回读共用 |
| 进程治理 | `crates/guardian/src/` | `lib.rs`（`ProcessGroupGuard` RAII + `Supervised` 组合句柄）/ `registry.rs`（活动进程组注册表 + Ctrl-C 守护 + 墙钟看门狗） |
| VM 内 init | `infra/init`、`infra/init-initramfs` | PID 1 脚本；`/init-hooks.sh` 为 builder 注入点 |
| 测试用例 | `infra/testcases/` | 独立 workspace（`Cargo.toml` 在目录根）：`testfw`（std）框架 + 用例 crate（C 测试体在 crate 的 `c/`，经 build.rs+cc 编入同一二进制；C 侧宏在 `framework/include/testfw.h`，FFI 落回 testfw 计数）；musl 静态 ELF（零 rustflags，与 tools 同配方）；`/tests/` 自动发现 |
| VM 内工具 | `infra/tools/` | 独立 workspace（std Rust + **musl 静态**，与 testcases 分类正交）：`agent/` = virtuoso-agent（virtio-serial JSON 行协议，AI probe 的 guest 侧）；装 tools.img 的 `/bin/`（VM 内挂 `/tools`，init-hooks 注入 PATH），不进 `/tests/` 不参与判定 |
| skill | `devkit/skills/kernel-dev/`、`devkit/skills/kernel-virtuoso/` | `virtuoso skill install` 装入内核树（后者 = AI 数据接口集成） |
| 内核开发容器 | `devkit/docker/` | **macOS 内核供给**：Dockerfile.kernel（钉死工具链+clangd）+ kernel.sh 薄壳（clone/build/export 进 named volume，规避 APFS 大小写坑）+ devcontainer.json（VS Code 打开 volume 即 clangd 全量索引）；镜像由 `.github/workflows/kernel-builder.yml` 发 ghcr |
| 文档站 | `docs/`（含 `book.toml`） | **docs/ 是文档唯一事实来源**且文档站自包含其内（书根 = docs/，book.toml 的 src 指向自身）；`cli/docs.rs` 接线 `virtuoso docs`；push main 由 `.github/workflows/docs.yml` 构建发布 gh-pages（https://yxalix.github.io/virtuoso/），产物落 `target/book` |

## 运行工件（AI 分诊数据源）

每次 `virtuoso test` / `matrix` 写入 `target/runs/<unix_ms>-<arch>/`（保留最近 20 次；
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

`virtuoso.toml` 是唯一配置面；标量键优先级：**进程环境变量 > `virtuoso.toml`**
（同名键 env 覆盖 toml，便于临时改参不动文件）；未知键/非法类型解析期报错。
**未配置项以注释形式存在于仓库根的
`virtuoso.toml` 模板**（活动行 = 默认常规启动配置）。VM 能力按组件配置，每个
`[components.*]` 段可用 `enabled` / `require`（KO 依赖，条目 = conf 行
`"<module> [key=val ...]"`）/ `stage`（`boot`｜`runtime`，缺省 runtime）：

```toml
# virtuoso.toml 示例（完整注释模板见仓库根）
arch = "arm64"           # x86_64 | arm64 | riscv64
timeout_secs = 60        # 0 一律拒绝
smp = 8                  # 多节点 NUMA 时须被 nodes 整除（解析期校验）
auto_test = true
qemu_opts = ["-device ivshmem-plain,memdev=hostmem"]   # 透传兜底
# kernel_preset = "mainline"   # 预编 mainline mini 内核（fetch 后免内核树；与 kernel_path 互斥，preset 优先）

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
initramfs 的 modules-boot.conf 冻结基础集之后。`virtuoso probe` 恒开 agent 通道
（强制并入 virtio_console，不依赖组件开关）。

## 冻结的不变量（不要破坏）

1. **标记协议 v1**（`infra/init` 输出，架构文档附录 A）：
   `--- Running: X ---`、`PASSED:/FAILED: X`、`Test Results: N/M passed`、
   `TEST_COMPLETE: ALL TESTS PASSED|SOME TESTS FAILED`。改文本等于破坏所有下游解析。
2. **test 退出码**：0=通过、124=超时、其余=失败。
3. **argv 冻结**：`QemuInvocation::argv` 的输出冻结在**按宿主平台的双基线**上
   （`common::HostOs`；Linux=memfd 后端、macOS=ram 后端、HVF→`-accel hvf`），
   由 `crates/launcher/src/qemu.rs` 的 `argv_*` 单测显式钉死平台把守；
   人工复核用 `QEMU=echo virtuoso shell` 打印 argv。数据盘（tools.img 等，
   追加 `-drive …,if=virtio` → `/dev/vdb` 起）与 agent 通道属调用方增量：
   **缺省（无盘无 agent）argv 与所属平台的基线逐字一致**；块设备抽象统一为
   `DataDisk`（多 virtio-blk 数据盘）。
4. 测试必须静态链接（`-static`），禁止用 `|| true` 掩盖失败。
5. **配置优先级：进程环境变量 > `virtuoso.toml`**：同名标量键以进程环境变量
   为准（CI/命令行临时改参不动文件）；持久配置只写 `virtuoso.toml`。

## 详细文档

`docs/` 是文档唯一事实来源（mdBook 书根 = `docs/`：`docs/book.toml` 的 src 指向
自身，`virtuoso docs` 构建，push main 自动发布 gh-pages）；改动文档只动 `docs/`，
别处引用不复制内容。

- `docs/quick-start.md` —— 从零到第一个 verdict: passed（装依赖 → 构建内核 → 体检 → 首跑）
- `docs/architecture/` —— `overview.md`（总体架构/目录结构）、`crates.md`（核心 crate 设计）、`contracts.md`（冻结契约 + 标记协议 v1 冻结文本）
- `docs/components/` —— 组件机制 `overview.md` + 逐组件页（tools-disk / agent / vfio / numa / pmem）
- `docs/guide/` —— `configuration.md`（virtuoso.toml 全键）、`kernel-preset.md`（预编内核开箱路径与供给链）、`writing-tests.md`（用例编写：Rust 入口 + C 体 FFI）、`debugging.md`、`artifacts.md`（运行工件与跨 run 分析）、`ai-integration.md`（skill + probe）
- `docs/internals/boot-pipeline.md` —— 两段式引导逐行解读、资产供给链、"改哪个文件"手册
- `docs/cli-reference.md` —— 命令一览；`docs/troubleshooting.md` —— Symptom → Solution 速查；`docs/contributing.md` —— 贡献约定 + CI

## 已知环境怪癖（勿"修复"）

见 `docs/internals/boot-pipeline.md` 的怪癖表：空 `/dev` 回退 mknod、sysfs dev 属性
为空、virtio/ext4 必须 `=m` 进 initramfs、BusyBox 裁掉 `CONFIG_TC` 等——都是
openEuler 内核的实测行为，删掉对应 fallback 会重新踩坑。
