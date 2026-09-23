# Virtuoso — 架构与设计

> **内核 E2E 虚拟化测试装置的总体架构与模块设计**
>
> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

---

## 1. 设计原则

| 原则 | 含义 |
|---|---|
| **Rust workspace 是唯一行为权威** | 构建与运行的全部逻辑在类型化 crate 中，`Makefile` 只是转发壳 |
| **类型安全** | 架构矩阵、NUMA 拓扑、组件依赖全部强类型化，非法配置解析期报错（而非运行时） |
| **RAII 资源治理** | QEMU 进程组、临时目录在任何退出路径（错误、panic、Ctrl-C、看门狗）下被收割 |
| **协议稳定** | 串口标记协议 v1 与退出码语义冻结（§3.3 与附录 A），CI 与 AI 接口零感知演进 |
| **判定权威** | verdict 是唯一判定事实；exit code 只是接口契约（`-no-reboot` 下内核 panic 使 QEMU exit 0） |
| **AI 原生** | 结构化事件流（events.jsonl / agent-events.jsonl）+ 数据接口集成，串口原文仅作补充 |
| **单机自包含** | 宿主机只需 Rust 工具链与 QEMU；BusyBox 走预编译 Release 供给，按架构缓存 |

---

## 2. 总体架构

### 2.1 命名

**Virtuoso（演奏大师）**，一词三关：

* **virt** — arm64/riscv64 的 machine type 就是 `virt`：Virtuoso 是字面意义上的"virt 机器演奏大师"；
* **大师** — AI 代理正是演奏家：开机器、读串口、判生死、写测试；
* **通用词** — 国际通用、好记好念，内核测试领域无同名项目。

crate 以角色名词命名（common / builder / launcher / judge / guardian / tracker /
xtask），名字与职责一一对应：可以直接说"让 builder 重建 initrd"、"judge 在等
TEST_COMPLETE"。

入口 crate 保留 xtask 这个 workspace 任务器传统名（工作区别名 `cargo xtask` / `cargo v`），对外的规范二进制名是 virtuoso——指挥家本人：`cargo install --path xtask` 后，PATH 上的就是它。

### 2.2 架构图

```text
               +-------------------------------------------------------------+
               |                  Developer / CI Engine                      |
               |        virtuoso test   ·   Claude Code (AI Skill)        |
               +-------------------------------------------------------------+
                                              |
                                              v
+-----------------------------------------------------------------------------------+
| Host: Virtuoso Rust Workspace (Control Plane)                                     |
|                                                                                   |
|  +------------------------+      +------------------------+    +---------------+  |
|  |      virtuoso       | ---> |        builder         | -->| target/bins   |  |
|  |  (总编排 CLI)           |      | initrd/rootfs/tools.img |   | target/runs   |  |
|  +------------------------+      +------------------------+    +---------------+  |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+                                                      |
|  |        guardian        |   RAII：进程组注册表 · Ctrl-C 守护 · 墙钟看门狗        |
|  +------------------------+                                                      |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+      +------------------------+    +---------------+  |
|  |        launcher        | ---> |  qemu-system-<arch>    | -->| tools.img     |  |
|  | 启动 DSL · QEMU/Firecracker |  |  virt / q35 · KVM·GDB  |    | vfio-pci      |  |
|  +------------------------+      +------------------------+    +---------------+  |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+     +-------------------------------------+          |
|  |         judge          | --> |  events.jsonl（结构化事件流）          | --> AI   |
|  |  串口解析·标记对账·判定  |     +-------------------------------------+          |
|  +------------------------+                                                      |
+-----------------------------------------|-----------------------------------------+
                                          |
                               -nographic | -serial mon:stdio
                                          v
+-----------------------------------------------------------------------------------+
| Guest VM:  virt / q35  ·  SMP×NUMA  ·  initramfs + ext4 rootfs（BusyBox）          |
|                                                                                   |
|  [Linux Kernel (被测)] --PID1--> [init: mountfs → insmod modules.conf → /tests/*] |
|       |                                                                           |
|       +---> [test-xxx: PASS/FAIL/SKIP/INFO ... Test Results: N/M passed]          |
|       +---> [TEST_COMPLETE: ALL TESTS PASSED] --> poweroff                        |
+-----------------------------------------------------------------------------------+
```

### 2.3 Workspace 目录结构

```text
virtuoso/
├── Cargo.toml                  # Root Workspace
├── .cargo/config.toml          # virtuoso 别名
├── virtuoso.toml               # 唯一配置面（模板：活动行 = 缺省常规启动配置）
├── Makefile                    # 转发壳（make 旧习惯 → virtuoso）
├── book.toml                   # mdBook 配置（src = docs/）
├── xtask/
│   └── src/
│       ├── main.rs             # clap 子命令定义
│       ├── config.rs           # 类型化配置（virtuoso.toml 唯一配置面）
│       ├── cli/                # verify / doctor / build / vm / probe / docs / parity / mod（分发+解析 helpers）/ diagnostics
│       └── runs/               # rundir（run 目录、输出泵、verdict 落盘回读）+ render（triage/runs/cluster/suggest/replay 呈现）
├── crates/
│   ├── common/                 # 基础层（零依赖）：Arch 矩阵 / which / ELF / 内存单位 / 时间 / 人类可读大小
│   ├── builder/                # 构建器：镜像发现 / C+Rust 用例 / 模块清单 / busybox 供给 / cpio+ext4 组装 / verify 引擎
│   ├── launcher/               # 启动 DSL（qemu.rs）+ NUMA（numa.rs）+ Firecracker 后端（firecracker.rs）
│   ├── judge/                  # 标记协议解析与判定（lib.rs）+ verdict schema（report.rs）+ 退出码语义（exit.rs）
│   ├── guardian/               # 进程组 RAII（lib.rs）+ 注册表 / 看门狗 / Ctrl-C（registry.rs）
│   └── tracker/                # 跨 run 语义：失败指纹归一化 / 聚类 / flaky / 补丁↔测试映射
├── infra/                      # VM 内源资产（构建时注入镜像，git 跟踪）
│   ├── init                    # 测试 init（rootfs 的 PID 1）
│   ├── init-initramfs          # stage-1 init（initramfs 的 PID 1：mount root= → switch_root）
│   ├── modules-boot.conf       # 冻结 boot 基础模块集（virtio + ext4 及依赖）
│   ├── testcases/              # C 用例（CMake，-static）+ rust/（no_std 独立 workspace）
│   └── tools/                  # VM 内工具独立 workspace（std Rust + musl 静态；agent = virtuoso-agent）
├── skills/                     # kernel-dev + kernel-virtuoso（virtuoso skill install 装入内核树）
└── docs/                       # 文档唯一事实来源（mdBook → gh-pages）
```

### 2.4 CLI

`virtuoso` 是唯一 CLI 入口（`.cargo/config.toml` 别名，另有短别名 `v`）；
`Makefile` 的每个 target 一一转发到对应子命令，语义与退出码不变。

| 命令 | 层 | 说明 |
|---|---|---|
| `verify [--arch a] [--backend b]` | builder | 前置检查 + 类型化配置诊断；firecracker 追加 microVM preflight |
| `doctor [--arch a] [--backend b] [--json]` | builder | 同一检查引擎的 flutter-doctor 风格一屏体检（✓/✗/! 组件行）；报 ✗ 时用 verify 看全量 |
| `build` | builder | 重建 initrd.img / rootfs.img / tools.img |
| `busybox` | builder | 确保当前架构静态 BusyBox（Release 下载优先，源码兜底） |
| `clean` | builder | 清理生成镜像与暂存目录 |
| `shell [--kvm] [--backend b]` | launcher | 交互式 VM（BusyBox shell） |
| `debug` | launcher | 挂起启动 + GDB stub `:1234` |
| `test --timeout N [--arch a] [--replay-until-fail N] [--backend b]` | 全链路 | 构建 → 启动 → 判定 → 工件落盘；返场模式首个非 passed 即停 |
| `matrix [--arch a]` | launcher | 多架构矩阵（缺省三架构，宿主内串行） |
| `probe --cmd/--cmd-file [--json]` | launcher+judge | AI 交互通道：virtio-serial agent 命令批，结构化事件流 |
| `triage [--run id] [--json]` | runs | 最近（或指定）run 的分诊报告 |
| `runs [--json]` | runs | 历史运行列表 |
| `replay --log f [--json]` | judge | 任意串口日志的离线标记协议断言 |
| `cluster [--json]` | tracker | 跨 run 失败指纹聚类 + flaky 清单 + 首现 run |
| `suggest [--diff f] [--json]` | tracker | git diff 子系统路径 → 推荐最小测试集 |
| `skill install/uninstall` | xtask | AI skill 装入 / 移出内核树 |
| `docs [--serve] [--open]` | xtask | mdBook 文档构建到 target/book / 本地预览 |
| `parity <target>` | xtask | make 与 virtuoso 行为对照（退出码三态判定） |

---

## 3. 核心模块设计

### 3.1 common — 基础层

零依赖，承载横切类型：`Arch` 矩阵唯一事实来源（QEMU 二进制 / 内核镜像路径 /
console / machine / 交叉前缀）、`which` 与 ELF 探测、内存量解析、UTC 时间、
人类可读大小。所有 crate 只依赖 common，不互相倒挂。

### 3.2 类型化配置（xtask/src/config.rs）

`virtuoso.toml` 是唯一配置面：全局键（arch / timeout_secs / smp / backend /
auto_test / kernel_path / kernel_image / qemu / qemu_opts / firecracker_bin）+
`[components.*]` 组件段。标量键优先级：进程环境变量 > `virtuoso.toml`
（同名键 env 覆盖 toml，临时改参不动文件）；未知键 / 非法类型解析期报错。
仓库根的 `virtuoso.toml`
模板即缺省常规启动配置，可选能力以注释形式在场。

VM 能力按组件声明，每个组件段支持：

- `enabled` —— 开关（段缺省 = 各组件缺省：tools_disk 启用，其余关闭）
- `require` —— KO 依赖，条目 = conf 行 `"<module> [key=val ...]"`（token 原样透传 insmod）
- `stage` —— `boot`（root 挂载前）｜`runtime`（缺省）

组件全集：`tools_disk`（tools.img 数据盘 → /dev/vdb → /tools）、`agent`
（virtio-serial AI 通道）、`vfio`（PCI 直通 → `-device vfio-pci,host=<bdf>`）、
`numa`（多节点拓扑）、`pmem`（持久内存，DT 途径：arm64/riscv64 专有），
以及全局 `[busybox]` 版本段。

`ComponentPlan` 把启用组件的 require 并集（schema 固定顺序
tools_disk→agent→vfio→numa→pmem，按首 token 去重保首个）按 stage 分区，
builder 据此生成 rootfs `/lib/modules/modules.conf`（runtime）并把 boot 条目
追加到 initramfs 冻结基础集之后。`virtuoso probe` 恒开 agent 通道（强制并入
virtio_console，不依赖组件开关）；firecracker 后端不支持 agent 与 pmem
（启用时 WARN 忽略）。

### 3.3 builder — 构建器

* **BusyBox 四层供给**：显式 URL（`BUSYBOX_DL_URL`）→ `gh release download`（`busybox-<ver>-linux-<arch>`，自带认证）→ 直链 wget（ELF 魔数校验）→ 源码归档兜底（busybox.net tarball → GitHub mirror）。产物按架构缓存于 `target/build/busybox/bin/`。
* **模块清单生成（modconf）**：boot = `modules-boot.conf` 冻结基础集 + 组件 `stage="boot"` 附加；runtime = 启用组件 require 并集生成的 `modules.conf`。按清单顺序 insmod（依赖手工排序，缺 `.ko` 构建期报错）。
* **C 用例**：CMake 构建，`-static -O2 -Wall` 冻结（VM 内无动态加载器），按 `Arch` 交叉前缀选择编译器。
* **no_std Rust 用例**：`infra/testcases/rust/` 独立 workspace，stable 工具链 + `#![no_std] #![no_main]` 自定义 `_start`，write/exit_group 裸 syscall，`-static -nostdlib -nostartfiles -no-pie`；宿主缺对应架构 rust-std 时显式 WARN 跳过（构建失败仍然报错）。
* **tools.img**：`infra/tools/`（std Rust + musl 静态）装入 ext4 数据盘 `/bin/`（卷标 `tools`），rootfs 不装工具；工具供给缺失时不产出 tools.img、不注入挂载 hook（降级显式非掩盖）。
* **cpio + ext4 组装**：initramfs（cpio newc）+ rootfs/tools（ext4）。
* **InitHook 注入点**：builder 生成 rootfs 内 `/init-hooks.sh`（init 侧守卫 source，位于 devtmpfs 挂载后、insmod/agent 拉起前），tools 盘挂载 + PATH 注入即走此通道；VM 内定制（如 hugetlbfs 预分配）写 hook 片段即可。
* **verify 检查引擎**：前置检查 + 类型化配置诊断（工具链 / 内核镜像 / QEMU / 模块 / BusyBox 缓存 / 组件状态）。

### 3.4 launcher — 启动 DSL 与双后端

`QemuInvocation`（`launcher::qemu`）强类型封装全部启动形态：machine（virt/q35）、
kernel、`-smp` + NUMA 拓扑（每节点一个 socket，单节点不传 `-numa`）、加速
（KVM 仅同构，交叉回退 TCG 并 WARN）、`-nographic -serial mon:stdio`、多
virtio-blk 数据盘（`DataDisk`，追加顺序决定 guest 内 `/dev/vdb` 起，rootfs 恒
`/dev/vda`）、GDB stub（`-S -gdb tcp::1234`）、agent 串口（virtio-serial）、
pmem（DT 补丁三件套）、`qemu_opts` 原样透传、cmdline
`console=<serial> root=/dev/vda rw init=/init loglevel=8 [auto_test]`。

**argv 冻结基线**：缺省（无数据盘、无 agent）时 `QemuInvocation::argv` 输出与
既定基线逐字一致，由单测把守；人工复核用 `QEMU=echo virtuoso shell` 打印。
数据盘与 agent 通道属调用方增量，追加在基线之后。

**pmem（DT 途径，arm64/riscv64）**：从 guest RAM 顶部挖出 `size` 区域 ——
dumpdtb 生成设备树 + fdtput 注入 `pmem-region` 节点（of_pmem 绑定，免 NFIT/EFI
依赖），cmdline 追加 `mem=<总内存 − pmem 区>` 把区间排除出线性内存模型，
`devm_memremap_pages` 才能建 ZONE_DEVICE；主内存后端换宿主文件
（memory-backend-file，guest 写入持久落盘）。三件套落 `target/build/pmem/`。

**Firecracker 后端**（`launcher::firecracker`）：config JSON（v1 API 冻结字段）+
API 逐 PUT 序列；多 drive 复用 `DataDisk`；`preflight` 硬校验（x86_64/aarch64、
KVM 必需、aarch64 内核须 ELF、无 initramfs 引导要求 virtio/virtio-blk/ext4/串口
`=y`），每项给可操作诊断。`spawn_supervised` 一体登记 guardian 监管。

### 3.5 judge — 判定引擎

**串口标记协议 v1（冻结）**：

| 标记 | 含义 | judge 行为 |
|---|---|---|
| `[PASS] / [FAIL] / [SKIP] / [INFO]` | 单断言行 | 逐行解析为结构化事件 |
| `Test Results: N/M passed` | 单二进制汇总 | 校验 N==M，否则 fail-fast |
| `TEST_COMPLETE: ALL TESTS PASSED` | 全局成功 | verdict = passed，等待 poweroff |
| `TEST_COMPLETE: SOME TESTS FAILED` | 全局失败 | verdict = failed |
| 退出码 124（137 归一为 124） | 墙钟超时 | verdict = timeout，收割进程组 |

**判定 = 标记对账 + 退出码**。`judge::Verdict` 八态：`passed` / `failed` /
`timeout` / `panic` / `incomplete` / `interrupted` / `build_failed` / `unknown`。
关键防护：`-no-reboot` 下内核 panic 使 QEMU 以 exit 0 退出——以
`TEST_COMPLETE` 标记与退出码对账识破假通过，panic/oops 独立成档；
`exit 0` 但标记协议未走完 → `incomplete`，按失败处理。

**产物**（`target/runs/<unix_ms>-<arch>/`，保留最近 20 次）：`serial.log`、
`qemu-stderr.log`、`build.log`、`events.jsonl`（逐行结构化事件：test_start /
test_end / assert / summary / marker / panic / oops / run_end）、
`verdict.json`（`VerdictReport`：汇总判定 + 运行指纹——内核 mtime/大小、QEMU
版本、拓扑、超时；构造与回读共用同一 serde schema）。退出码语义唯一表在
`judge::exit`（0=通过、124=超时、其余=失败）。

### 3.6 guardian — 进程治理

* **`ProcessGroupGuard`（RAII）**：QEMU 进程组收割，Drop / 超时 / Ctrl-C 三路径统一 KILL；pgid=0 惰性登记防自杀。
* **`registry`**：活动进程组全局注册表 + Ctrl-C 守护（`install_ctrlc_guard`，xtask 入口装载）+ 墙钟看门狗。`Supervised` 是"登记 + 收割守卫"组合句柄，与 launcher 的 `spawn_supervised` 消除 spawn 样板。

`virtuoso test` 中途被 Ctrl-C 打断时：收割 QEMU 进程组 → 落盘已产出的 run 工件 → 以 130 退出，宿主机不残留虚拟化进程。

### 3.7 tracker — 跨 run 语义

输入是最小摘要 `RunSummary`（`From<VerdictReport>` 投影，verdict schema 演进不外溢），IO 留在 runs 层：

* **失败指纹** = verdict 类 + 归一化证据（剥离内核时间戳、数字折叠为 `N`；panic/oops 优先 → timeout → 失败测试集）；
* **聚类**（`cluster`）：跨 run 指纹分组，输出 flaky 用例清单（flaky 判定只信 passed/failed 的 run）与每类失败首现 run；
* **补丁↔测试映射**（`suggest`）：git diff 的子系统路径前缀 → 最小测试集（缺省规则表 `DEFAULT_RULES`，可由 diff 输入覆盖）；
* **返场**（`test --replay-until-fail N`）：对可疑 flaky 场景自动返场，首个非 passed verdict 即停。

### 3.8 AI 数据接口

| 能力 | 输入 | 输出 | 对接点 |
|---|---|---|---|
| **测试脚手架生成** | 自然语言描述 / git diff | C 或 no_std Rust 用例 + 构建注册 | builder 编译即用 |
| **串口日志分诊** | `events.jsonl` / `verdict.json` | 根因假设 + 建议复现命令 | `virtuoso triage` |
| **失败指纹聚类** | 跨 run 的 verdict/事件流 | flaky 清单 + 失败首现 run | `virtuoso cluster`（tracker） |
| **补丁↔测试映射** | `git diff` + 子系统路径 | 推荐最小测试集 | `virtuoso suggest`（tracker） |
| **VM 内交互探测** | shell 命令批 | 结构化事件流（`agent-events.jsonl`） | `virtuoso probe`（virtio-serial + tools/virtuoso-agent，JSON 行协议） |

架构约束（安全边界）：

* AI **只读分析**测试产物与内核日志；对内核源码的任何修改必须人工确认后由开发者执行；
* `events.jsonl` 是 AI 的唯一结构化事实源，串口原文仅作补充上下文，保证分诊可回溯；
* skill 与 harness 的接口是冻结契约：标记协议 v1 不变，skill 只依赖协议与工件 schema，不依赖实现。

---

## 4. 测试用例框架

C 与 no_std Rust 两条路径并存，**协议 v1 对两者一视同仁**（同一串口协议、同一套
PASS/FAIL 宏语义）：

* **C**：共享 `main.c`（`run_tests()` 入口 + 汇总打印）、`test_common.h` 宏与计数器；
  新二进制注册进 CMakeLists，`-static` 冻结。
* **Rust**：`testfw` no_std 框架（宏与 C `test_common.h` 一一对齐，`run_and_exit`
  对齐共享 `main.c` 语义），裸 syscall 静态 ELF。

`init` 自动发现 rootfs `/tests/` 下全部二进制——新增用例零接线。编写步骤见
[使用指南](user-guide.md)。

---

## 5. CI/CD

`.github/workflows/` 三条流水线：

| 工作流 | 内容 |
|---|---|
| `virtuoso-ci.yml` | build / clippy（`-D warnings`）/ unit test；E2E 走自建 runner（openEuler 宿主 + KVM），手动 `workflow_dispatch` 触发，失败时上传 `target/runs/` 整体工件 |
| `busybox-release.yml` | 三架构静态 BusyBox 预编译发布 GitHub Release；构建环境钉死 `ubuntu:22.04` 容器（busybox 1.36.1 的 tc applet 无法在内核头文件 ≥ 6.8 下编译，交叉 gcc 行为随发行版漂移）；交叉编译必须显式安装 `libc6-dev-<arch>-cross`（`--no-install-recommends` 会漏装） |
| `docs.yml` | mdBook 构建 docs/ → GitHub Pages（gh-pages） |

---

## 6. 冻结契约

| 维度 | 契约 |
|---|---|
| 串口标记协议 | v1 冻结（附录 A）：改动文本等于破坏所有下游解析 |
| 退出码 | 0=通过、124=超时（137 归一）、其余=失败；唯一表在 `judge::exit` |
| QEMU argv | 缺省（无数据盘、无 agent）输出与冻结基线逐字一致，`argv_*` 单测把守 |
| 静态链接 | 测试必须 `-static`；禁止 `|| true` 掩盖失败 |
| 配置优先级 | 标量键：进程环境变量 > `virtuoso.toml`（同名键 env 覆盖） |
| AI 接口 | skill 只依赖标记协议 v1 与工件 schema（verdict.json / events.jsonl），不依赖 harness 内部实现 |

---

## 附录 A：串口标记协议 v1（冻结文本）

```text
# 单断言行（测试二进制内打印）
[PASS] <message>
[FAIL] <message>
[SKIP] <message>
[INFO] <message>

# 单二进制汇总（共享 main.c 打印）
Test Results: <N>/<M> passed

# 全局终止标记（harness 判定依据）
TEST_COMPLETE: ALL TESTS PASSED     # → exit 0
TEST_COMPLETE: SOME TESTS FAILED    # → exit 1

# 超时（宿主侧判定）
exit 124   # wallclock timeout（内核挂死 / runaway loop）
```
