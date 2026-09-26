# 总体架构

> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

## 设计原则

| 原则 | 含义 |
|---|---|
| **Rust workspace 是唯一行为权威** | 构建与运行的全部逻辑在类型化 crate 中，CLI 是唯一操作面 |
| **类型安全** | 架构矩阵、NUMA 拓扑、组件依赖全部强类型化，非法配置解析期报错（而非运行时） |
| **RAII 资源治理** | QEMU 进程组、临时目录在任何退出路径（错误、panic、Ctrl-C、看门狗）下被收割 |
| **协议稳定** | 串口标记协议 v1 与退出码语义冻结（[冻结契约](contracts.md)），CI 与 AI 接口零感知演进 |
| **判定权威** | verdict 是唯一判定事实；exit code 只是接口契约（`-no-reboot` 下内核 panic 使 QEMU exit 0） |
| **AI 原生** | 结构化事件流（events.jsonl / agent-events.jsonl）+ 数据接口集成，串口原文仅作补充 |
| **单机自包含** | 宿主机只需 Rust 工具链与 QEMU；BusyBox 走预编译 Release 供给，按架构缓存 |

## 命名

**Virtuoso（演奏大师）**，一词三关：

* **virt** — arm64/riscv64 的 machine type 就是 `virt`：Virtuoso 是字面意义上的"virt 机器演奏大师"；
* **大师** — AI 代理正是演奏家：开机器、读串口、判生死、写测试；
* **通用词** — 国际通用、好记好念，内核测试领域无同名项目。

crate 以角色名词命名（common / builder / launcher / judge / guardian / tracker），
名字与职责一一对应：可以直接说"让 builder 重建 initrd"、"judge 在等
TEST_COMPLETE"。

规范二进制 virtuoso 是根包本体（`src/main.rs`）——指挥家本人：`cargo install --path .` 后，PATH 上的就是它。全部命令见 [CLI 参考](../cli-reference.md)。

## 架构图

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
|  |      启动 DSL · QEMU       |  |  virt / q35 · KVM·GDB  |    | vfio-pci      |  |
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

各 crate 的内部设计见[核心 crate 设计](crates.md)。

## Workspace 目录结构

```text
virtuoso/
├── Cargo.toml                  # Root Workspace + virtuoso 根包（src/main.rs）
├── virtuoso.toml               # 唯一配置面（模板：活动行 = 缺省常规启动配置）
├── src/
│   ├── main.rs                 # clap 子命令定义
│   ├── config.rs               # 类型化配置（virtuoso.toml 唯一配置面）
│   ├── cli/                    # verify / doctor / build / vm / probe / docs / mod（分发+解析 helpers）/ diagnostics
│   └── runs/                   # rundir（run 目录、输出泵、verdict 落盘回读）+ render（triage/runs/cluster/suggest/replay 呈现）
├── crates/
│   ├── common/                 # 基础层（零依赖）：Arch 矩阵 / which / ELF / 内存单位 / 时间 / 人类可读大小
│   ├── builder/                # 构建器：镜像发现 / C+Rust 用例 / 模块清单 / busybox 供给 / cpio+ext4 组装 / verify 引擎
│   ├── launcher/               # 启动 DSL（qemu.rs）+ NUMA（numa.rs）
│   ├── judge/                  # 标记协议解析与判定（lib.rs）+ verdict schema（report.rs）+ 退出码语义（exit.rs）
│   ├── guardian/               # 进程组 RAII（lib.rs）+ 注册表 / 看门狗 / Ctrl-C（registry.rs）
│   └── tracker/                # 跨 run 语义：失败指纹归一化 / 聚类 / flaky / 补丁↔测试映射
├── infra/                      # VM 内源资产（构建时注入镜像，git 跟踪）
│   ├── init                    # 测试 init（rootfs 的 PID 1）
│   ├── init-initramfs          # stage-1 init（initramfs 的 PID 1：mount root= → switch_root）
│   ├── modules-boot.conf       # 冻结 boot 基础模块集（virtio + ext4 及依赖）
│   ├── testcases/              # 用例 workspace（testfw std 框架 + 用例 crate，C 体经 build.rs+cc 编入，musl 静态）
│   └── tools/                  # VM 内工具独立 workspace（std Rust + musl 静态；agent = virtuoso-agent）
├── devkit/                     # 内核开发外围工具（非运行路径，均为源资产）
│   ├── docker/                 # 容器化内核开发环境（macOS 内核供给，ghcr 镜像）
│   └── skills/                 # kernel-dev + kernel-virtuoso（virtuoso skill install 装入内核树）
└── docs/                       # 文档唯一事实来源（mdBook → gh-pages；book.toml 内嵌，书根 = docs/）
```

两段式引导（initramfs → rootfs）的逐行解读与构建流水线见
[两段式引导与构建流水线](../internals/boot-pipeline.md)。
