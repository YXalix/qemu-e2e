··# Virtuoso — 内核 E2E 虚拟化测试基础设施现代化方案

> **qemu-e2e 的 Rust 全面改造设计文档 · v2.0**
>
> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

---

## 1. 文档概述与背景

### 1.1 qemu-e2e 现状盘点（主体特性）

`qemu-e2e` 是一个直接放进内核源码树（`kernel/qemu-e2e/`）的端到端测试框架，用一条命令回答：**"does my patch actually work?"**。其主体特性如下：

| # | 特性 | 现状实现 |
|---|------|----------|
| F1 | **一命令测试回路** | `make qemu-test QEMU_TIMEOUT=30`：重建 initrd → 启动 QEMU → 串口断言 → 非零退出码 |
| F2 | **直启内核** | 直接 `-kernel` 启动 freshly-built 内核，无发行版 userland、无 overlay 镜像、无刷机 |
| F3 | **多架构矩阵** | `arm64`（默认，`qemu-system-aarch64` + `virt` + `ttyAMA0`）、`x86_64`（`bzImage` + `q35` + `ttyS0`）、`riscv64`（`virt` + `ttyS0`），支持任意交叉组合并告警 |
| F4 | **静态 C 测试 + BusyBox initramfs** | 静态链接测试二进制（`-static -O2 -Wall`）装入 BusyBox 1.36.1 initramfs；通过 syscall/ioctl/`/proc`/`/sys`/`/dev` 驱动被测内核 |
| F5 | **内核模块加载** | `modules.conf` 声明式：有序 `insmod`、支持加载参数、依赖手工排序、缺 `.ko` 构建即报错 |
| F6 | **NUMA 拓扑** | `NUMA_NODES × NUMA_MEMORY`（默认 2 节点 × 1G）、`SMP=8` vCPU 均分到各节点，单节点时不传 `-numa` |
| F7 | **可调试性** | KVM 加速（同构时）、GDB stub（`:1234`，配 `gdb-multiarch vmlinux`）、交互 BusyBox shell、可选 512M NVMe（`disk.qcow2` → `/dev/nvme0n1`）、PCI 直通（`QEMU_OPTS='-device vfio-pci,...'`） |
| F8 | **CI 友好** | 墙钟超时（`timeout --signal=KILL`，超时退出码 124）、`TEST_COMPLETE` 标记协议、进程组收割 |
| F9 | **串口标记协议** | `[PASS]/[FAIL]/[SKIP]/[INFO]`、`Test Results: N/M passed`、`TEST_COMPLETE: ALL TESTS PASSED / SOME TESTS FAILED` |
| F10 | **可复现** | 同一 `.env` + 同一内核 → 同一次 boot；BusyBox 静态缓存 |
| F11 | **AI 集成** | `kernel-dev` Claude Code skill：`make install-skill` 装入内核树，指导 AI 驱动测试回路、写测试、解析串口、分诊失败 |
| F12 | **VM 内定制** | PID 1 为 `infra/init` POSIX 脚本：挂载 proc/sysfs/devtmpfs/tmpfs/debugfs/devpts/shm → 有序 insmod → 自动测试后 poweroff，或落入交互 shell |

### 1.2 改造动因（痛点）

随着 F1–F12 特性面扩大，`Makefile + POSIX Shell + C Testcase` 工具链显露出系统性瓶颈：

* **进程管理脆弱**：`qemu-test` 靠 Makefile 内联的 PID 文件 + `kill -- -$$QEMU_PGID` 收割进程组，超时/异常路径下 QEMU 与 `timeout` 的信号竞争靠运气；`Ctrl-C` 半途打断会残留 QEMU 子进程。
* **配置无类型**：`.env` 是纯字符串，`SMP` 必须被 `NUMA_NODES` 整除、`QEMU_TIMEOUT=0` 必须被拒绝这类约束只能在运行时用 shell 判断，错误发现太晚、报错不精确。
* **跨架构逻辑分散**：架构表（QEMU 二进制 / 内核镜像路径 / console 设备 / machine 类型）散落在 3 个 shell 脚本中，新增架构要改多处。
* **Shell 不可重构**：`init`（PID 1）与 `build-initrd.sh` 受 BusyBox `sh` 方言限制，无测试、无重构工具、无调试手段；启动定制（如 hugetlbfs 预分配）只能手改脚本。
* **AI 层浮于表面**：`kernel-dev` skill 只有自然语言提示词接口，串口日志是纯文本流，AI 分诊缺乏结构化数据（结构化事件、失败指纹、复现命令），无法稳定集成到自动化。

### 1.3 改造目标

全面采用 **Rust 语言生态**，将 qemu-e2e 的全部主体特性收敛进 **Virtuoso Workspace**：

1. **类型安全**：架构矩阵、NUMA 拓扑、QEMU 参数全部强类型化，非法配置在解析期报错。
2. **自愈式资源治理**：RAII 保证 mount/QEMU 进程/临时目录在任何退出路径（含 panic、信号）下被收割。
3. **协议稳定**：串口标记协议与退出码语义**原样保留**（v1 冻结），现有 CI 与 AI skill 零改动迁移。
4. **AI 原生**：为 AI 提供结构化事件流与分诊接口，从"提示词集成"升级为"数据接口集成"。
5. **原生跨平台**：单文件静态编译分发，宿主机只需 Rust 工具链与 QEMU。

---

## 2. 命名与整体架构

### 2.1 命名：Virtuoso

**Virtuoso（演奏大师）**，一词三关：

* **virt** — qemu-e2e 的 arm64/riscv64 machine type 就是 `virt`：Virtuoso 是字面意义上的"virt 机器演奏大师"；
* **大师** — 新方案中 AI 代理正是演奏家：开机器、读串口、判生死、写测试；
* **通用词** — 国际通用、好记好念，内核测试领域无同名项目。

子模块最初采用音乐系命名家族；**Phase 3 重构起改为角色名词**（common / builder /
launcher / judge / guardian / tracker / xtask，见 §2.3 与附录 B 决策记录），名字与
职责一一对应，团队沟通时可直接说"让 builder 重建 initrd"、"judge 在等 TEST_COMPLETE"。

### 2.2 整体架构

```text
               +-------------------------------------------------------------+
               |                  Developer / CI Engine                      |
               |        cargo xtask test   ·   Claude Code (AI Skill)        |
               +-------------------------------------------------------------+
                                              |
                                              v
+-----------------------------------------------------------------------------------+
| Host: Virtuoso Rust Workspace (Control Plane)                                     |
|                                                                                   |
|  +------------------------+      +------------------------+    +---------------+  |
|  |      cargo xtask       | ---> |        builder         | -->| target/bins   |  |
|  |  (maestro: 总编排 CLI)  |      | initrd/内核镜像/C 用例  |    | target/initrd |  |
|  +------------------------+      +------------------------+    +---------------+  |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+      +------------------------+                      |
|  |        guardian        | ---> |  nix / tempfile / pty  |   (RAII Guards)      |
|  | mount·进程组·临时目录    |      +------------------------+                      |
|  +------------------------+                                                      |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+      +------------------------+    +---------------+  |
|  |        launcher        | ---> |     qemu-system-<arch> | -->| disk.qcow2    |  |
|  | 多架构矩阵·启动 DSL      |      |  virt / q35 · KVM·GDB  |    | vfio-pci      |  |
|  +------------------------+      +------------------------+    +---------------+  |
|              |                                                                   |
|              v                                                                   |
|  +------------------------+     +-------------------------------------+          |
|  |         judge          | --> |  events.jsonl（结构化事件流）          | --> AI   |
|  | 串口监听·断言·分诊数据源  |     +-------------------------------------+          |
|  +------------------------+                                                      |
+-----------------------------------------|-----------------------------------------+
                                          |
                               -nographic | -serial mon:stdio
                                          v
+-----------------------------------------------------------------------------------+
| Guest VM:  virt / q35  ·  SMP×NUMA  ·  BusyBox initramfs                          |
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
├── .cargo/
│   └── config.toml             # cargo xtask 别名
├── Cargo.toml                  # Root Workspace
├── xtask/                      # CLI 入口（只做分发、配置投影与呈现；Phase 3 重构后）
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # CLI 入口（clap 子命令定义）
│       ├── config.rs           # 类型化配置（.env 兼容 + virtuoso.toml）
│       ├── cli/                # 命令实现：mod（分发+解析 helpers）/ verify / build / vm / parity / diagnostics
│       └── runs/               # 运行工件：rundir（目录/泵/落盘回读）+ render（triage/runs/cluster/suggest/replay）
├── crates/
│   ├── common/                 # 基础层（零依赖）：Arch 矩阵 / which / ELF / 内存单位 / 时间 / 人类可读大小
│   ├── builder/                # 构建器（内核镜像发现 / C 用例编译 / initrd 组装 / verify 检查引擎）
│   ├── launcher/               # 多架构矩阵 + QEMU 启动 DSL（qemu.rs）+ Firecracker 后端（firecracker.rs）
│   ├── judge/                  # 标记协议解析与判定 + verdict.json schema（report.rs）+ 退出码语义（exit.rs）
│   ├── guardian/               # 进程组 RAII（lib.rs）+ 注册表/看门狗/Ctrl-C（registry.rs）
│   └── tracker/                # 跨 run 聚类 / flaky / 补丁↔测试映射（原 encore 域）
├── skills/
│   └── kernel-virtuoso/SKILL.md  # AI 组件（kernel-dev 的进化版）
└── testcases/                  # C 测试用例（沿用 qemu-e2e 约定，Phase 3 增加 no_std Rust）
    ├── CMakeLists.txt
    └── src/
        ├── main.c              # 共享入口：run_tests()
        ├── test_common.h/c     # PASS/FAIL/SKIP/INFO 宏 + 计数器
        └── test_example.c
```

### 2.4 CLI 映射：Makefile → `cargo xtask`

`.cargo/config.toml`：

```toml
[alias]
xtask = "run --package xtask --"
v = "xtask"          # 短别名，顺便致敬串口终端 VT100
```

qemu-e2e 全部 8 个 target 一一对应，语义与退出码不变：

| qemu-e2e（Make） | Virtuoso（cargo xtask） | 说明 |
|---|---|---|
| `make verify` | `cargo xtask verify` | 前置检查：工具链、内核镜像、QEMU、模块、BusyBox 缓存 |
| `make initrd` | `cargo xtask build` | builder：C 用例编译 + initrd 组装 |
| `make qemu` | `cargo xtask shell` | 交互式 BusyBox shell（TCG） |
| `make qemu-kvm` | `cargo xtask shell --kvm` | KVM 加速（仅同构） |
| `make qemu-debug` | `cargo xtask debug` | 挂起启动 + GDB stub `:1234` |
| `make qemu-test QEMU_TIMEOUT=30` | `cargo xtask test --timeout 30` | CI 模式；`--timeout 0` 一律拒绝 |
| `make disk` | `cargo xtask disk` | 512M `disk.qcow2`（幂等，已存在则跳过） |
| `make clean` | `cargo xtask clean` | 清理生成物 |
| `make install-skill` | `cargo xtask skill install` | AI skill 装入内核树 |
| —（新增） | `cargo xtask matrix` | launcher：多架构矩阵批量测试 |
| —（新增） | `cargo xtask triage` | AI 分诊：events.jsonl → 根因分析报告 |
| —（新增） | `cargo xtask replay <log>` | judge：串口日志回放断言 |

---

## 3. 核心模块详细设计

> 命名注记（Phase 3 重构）：本节成文于音乐系命名时期，文中 overture / ensemble /
> auditor / coda / encore 依次对应现行 builder / launcher / judge / guardian /
> tracker；模块划分与设计意图不变，另有 common 基础层承载横切类型（见附录 B）。

### 3.1 类型化配置（xtask/src/config.rs）

qemu-e2e 的 `.env` 全部变量收敛为强类型结构；Phase 1–2 保持 `.env` 兼容读取，Phase 2 起推荐 `virtuoso.toml`：

```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VirtuosoConfig {
    /// 内核源码树；缺省自动探测：可执行文件目录的上一级（qemu-e2e 语义）
    pub kernel_path: PathBuf,
    pub arch: Arch,
    /// 墙钟超时（秒）。0 一律拒绝 —— qemu-e2e 语义保留
    pub timeout_secs: u64,
    /// 总 vCPU，须被 numa.nodes 整除（否则配置解析期报错，而非运行时）
    pub smp: u32,
    pub numa: NumaConfig,
    /// 覆盖 qemu-system-<arch> 路径
    pub qemu: Option<PathBuf>,
    /// 透传 QEMU 参数，如 -device vfio-pci,host=0000:01:00.0
    pub qemu_opts: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch { Arm64, X86_64, Riscv64 }

#[derive(Debug, Clone, Deserialize)]
pub struct NumaConfig {
    /// 每节点内存；总内存 = memory_per_node × nodes
    pub memory_per_node: ByteSize,
    /// 1 = 单节点（不传 -numa），>1 = 每节点一个 socket
    pub nodes: u32,
}
```

架构矩阵成为唯一事实来源（qemu-e2e 中散落 3 个脚本的表格收敛于此）：

```rust
impl Arch {
    pub fn qemu_bin(self) -> &'static str {
        match self {
            Self::Arm64   => "qemu-system-aarch64",
            Self::X86_64  => "qemu-system-x86_64",
            Self::Riscv64 => "qemu-system-riscv64",
        }
    }
    pub fn kernel_image(self, kernel: &Path) -> PathBuf {
        match self {
            Self::Arm64   => kernel.join("arch/arm64/boot/Image"),
            Self::X86_64  => kernel.join("arch/x86/boot/bzImage"),
            Self::Riscv64 => kernel.join("arch/riscv/boot/Image"),
        }
    }
    pub fn console(self) -> &'static str {
        match self { Self::Arm64 => "ttyAMA0", _ => "ttyS0" }
    }
    pub fn machine(self) -> &'static str {
        match self { Self::X86_64 => "q35", _ => "virt" }
    }
    /// 交叉编译前缀（overture 编译 C 用例用）
    pub fn cross_prefix(self) -> Option<&'static str> {
        match self {
            Self::Arm64   => Some("aarch64-linux-gnu-"),
            Self::Riscv64 => Some("riscv64-linux-gnu-"),
            Self::X86_64  => None,
        }
    }
}
```

### 3.2 overture — 构建器（initrd / C 用例 / 模块）

接管 `build-initrd.sh` 与 `testcases/CMakeLists.txt` 的职责：

* **内核镜像发现与校验**：按 `Arch::kernel_image` 定位镜像，缺失时给出可执行建议（"先在 `$KERNEL_PATH` 执行 `make -j$(nproc)`"）；
* **C 用例增量编译**：保留 `-static -O2 -Wall`（VM 内无动态加载器，`-static` 不可放松），按 `cross_prefix` 选择交叉编译器，产物输出 `target/<arch>/tests/`；
* **initrd 组装**：BusyBox **预编译优先供给**（`.github/workflows/busybox-release.yml` 在 GitHub Release 发布 `busybox-<ver>-linux-<arch>`，arm64/x86_64/riscv64 三架构；`fetch-busybox.sh` 按 ARCH 缓存于 `infra/busybox/bin/`，下载失败或离线时回退源码编译并告警）→ 复制 `modules.conf` 声明的 `.ko`（按声明顺序，缺 `.ko` 构建期报错）→ 注入 `/tests/` 二进制与 `init`；
* **init 的 Rust 化**：PID 1 的挂载序列（proc/sysfs/devtmpfs/tmpfs/debugfs/devpts/shm）与 `auto_test` 流程由模板生成，启动定制（如 hugetlbfs 预分配）从"手改脚本"升级为声明式 hook：

```rust
// crates/overture/src/init_hooks.rs
pub enum InitHook {
    /// VM 内幂等 shell 片段，插入 mount 之后、insmod 之前
    Shell { name: String, script: String },
}

// 例：预分配 hugepages（qemu-e2e README 中的定制示例 → 声明式）
let hugepages = InitHook::Shell {
    name: "hugetlbfs".into(),
    script: r#"
        mkdir -p /mnt/huge
        mount -t hugetlbfs nodev /mnt/huge
        echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
    "#
    .into(),
};
```

### 3.3 ensemble — 多架构矩阵与 VM 启动 DSL

强类型 DSL 封装 qemu-e2e 的全部启动形态，取代 `run-qemu.sh` 的分支拼串：

```rust
// crates/ensemble/src/lib.rs
let vm = QemuInvocation::new(cfg.arch, cfg.qemu.as_deref())?
    .machine(cfg.arch.machine())              // virt / q35
    .kernel(&image)
    .cpu(cfg.smp, &cfg.numa)                  // -smp 8 + 每节点 -numa node,mem=1G,cpus=...
    .accel(if kvm_available && is_native {
        Accel::Kvm                            // qemu-kvm：仅同构可用
    } else {
        Accel::Tcg                            // 交叉架构回退 TCG，并 WARN
    })
    .serial(Channel::MonStdio)                // -nographic -serial mon:stdio
    .nvme(disk.as_deref())                    // 可选：disk.qcow2 → /dev/nvme0n1
    .gdb_stub(mode == Debug)                  // -S -gdb tcp::1234
    .extra(&cfg.qemu_opts)                    // vfio-pci 直通等原样透传
    .cmdline(format!(
        "console={} root=/dev/ram0 rw init=/init loglevel=8 auto_test",
        cfg.arch.console()
    ));

let session = vm.spawn().map_err(|e| match e {
    SpawnError::BinaryMissing(b) => anyhow!(
        "{b} not found; run `cargo xtask verify` for install hints"),
    other => anyhow!(other),
})?;
```

矩阵模式（`cargo xtask matrix`）对三架构并行调度（同宿主机串行执行避免资源争抢，CI 多 runner 则天然并行），汇总为一张 `Test Results` 总表。

### 3.4 auditor — 串口断言引擎与标记协议

**串口标记协议 v1（冻结）**：与 qemu-e2e 完全一致，现有 CI / skill 零改动。

| 标记 | 含义 | auditor 行为 |
|---|---|---|
| `[PASS] / [FAIL] / [SKIP] / [INFO]` | 单断言行 | 逐行解析为结构化事件 |
| `Test Results: N/M passed` | 单二进制汇总 | 校验 N==M，否则 fail-fast |
| `TEST_COMPLETE: ALL TESTS PASSED` | 全局成功 | 判定 verdict = Passed，等待 poweroff |
| `TEST_COMPLETE: SOME TESTS FAILED` | 全局失败 | 判定 verdict = Failed |
| 进程退出码 124/137 | 墙钟超时（内核挂死/死循环） | 判定 verdict = Timeout，收割进程组 |

```rust
// crates/auditor/src/lib.rs
pub enum Verdict {
    Passed { passed: u32, total: u32 },
    Failed { failed_cases: Vec<CaseRecord> },
    Timeout { after: Duration },
    BootFailure { tail: String },   // 内核 panic / 未到首个标记
}

pub struct AuditSession {
    pty: PtySession,                 // rexpect，订阅 -serial mon:stdio
    events: Vec<SerialEvent>,        // 全程留痕 → events.jsonl
}

impl AuditSession {
    /// 依次等待期望串口模式；全部命中且 TEST_COMPLETE=ALL → Passed
    pub fn run_and_assert(&mut self, expects: &[&str], timeout: Duration) -> Result<Verdict> {
        for pattern in expects {
            self.pty.exp_string(pattern)
                .map_err(|_| anyhow!("serial timeout waiting for `{pattern}`"))?;
        }
        self.wait_terminal(timeout)
    }
}
```

auditor 同时输出**结构化事件流**（AI 组件的数据源，见 §3.8）：

```jsonl
{"ts":"00:00:04.113","kind":"test_result","binary":"test-nvme","case":"blkid reads /dev/nvme0n1","status":"PASS"}
{"ts":"00:00:04.201","kind":"kernel_log","level":"warn","module":"nvme","text":"missing NVMe queue"}
{"ts":"00:00:05.872","kind":"summary","binary":"test-nvme","passed":3,"total":3}
{"ts":"00:00:06.000","kind":"verdict","result":"ALL_TESTS_PASSED"}
```

### 3.5 coda — RAII 资源治理

qemu-e2e Makefile 中两处最脆弱的代码（PID 文件 + `kill -- -$$QEMU_PGID`、以及未来 rootfs 挂载）由 RAII Guard 全面接管，任何退出路径（错误、panic、SIGINT/SIGTERM）均保证清理：

```rust
// crates/coda/src/lib.rs

/// 进程组收割：替代 Makefile 的 PID 文件 + kill -- -PGID hack
pub struct ProcessGroupGuard {
    pgid: u32,
    armed: bool,
}

impl ProcessGroupGuard {
    pub fn spawn(child: &mut Child) -> Result<Self> {
        let pgid = child.id();
        Ok(Self { pgid, armed: true })
    }
    /// 超时路径：先 SIGTERM，宽限期后 SIGKILL（qemu-e2e 的 KILL 语义保留）
    pub fn terminate(&mut self, grace: Duration) { /* ... */ }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = Command::new("kill").args(["-TERM", &format!("-{}", self.pgid)]).status();
        }
    }
}

/// loop 镜像安全挂载：panic 亦自动 umount -l
pub struct SafeMountGuard {
    mount_point: PathBuf,
    armed: bool,
}

impl Deref for SafeMountGuard {
    type Target = Path;
    fn deref(&self) -> &Path { &self.mount_point }
}
```

配合 panic hook 与 `ctrlc` 信号转发：`cargo xtask test` 中途被 `Ctrl-C` 打断时，coda 先收割 QEMU 进程组、再卸载挂载点、最后以 130 退出——宿主机不再残留任何虚拟化痕迹。

### 3.6 encore — 可复现与回放

* **指纹**：`(kernel Image hash, initrd hash, config digest, qemu version)` 四元组构成 run fingerprint，写入测试报告——"同一指纹必须同结局"是回归判定依据；
* **缓存**：BusyBox 静态二进制、按 arch 的测试产物缓存（继承 qemu-e2e 的 BusyBox 缓存策略）；
* **回放**：`cargo xtask replay <serial.log>` 把历史串口日志重新喂给 auditor 走一遍断言，用于离线复核与 AI 分诊训练样本；
* **返场**：`cargo xtask test --replay-until-fail N` 对可疑 flaky 用例自动返场 N 次，聚合 verdict。

### 3.7 AI 组件 — kernel-virtuoso

`skills/kernel-virtuoso/SKILL.md`（`kernel-dev` 的进化版，`cargo xtask skill install` 装入内核树）。AI 能力从"提示词集成"升级为**数据接口集成**：

| 能力 | 输入 | 输出 | 对接点 |
|---|---|---|---|
| **测试脚手架生成** | 自然语言描述 / git diff | `test_<name>.c` + CMakeLists 注册 | overture 编译即用 |
| **串口日志分诊** | `events.jsonl` | 根因假设 + 建议复现命令 | `cargo xtask triage` |
| **失败指纹聚类** | 跨 run 的 verdict/事件流 | flaky 用例清单与首现 commit | encore 指纹 |
| **补丁↔测试映射** | `git diff` + 子系统路径 | 推荐最小测试集（`matrix --subset`） | ensemble |

架构约束（安全边界）：

* AI **只读分析**测试产物与内核日志；对内核源码的任何修改必须人工确认后由开发者执行；
* auditor 的 `events.jsonl` 是 AI 的唯一结构化事实源，串口原文仅作为补充上下文，保证分诊可回溯；
* skill 与 harness 的协议是本方案的冻结接口之一：标记协议 v1 不变，skill 只依赖协议不依赖实现。

---

## 4. 测试用例框架演进

**Phase 1–2（C 框架原样保留）**：`test_common.h` 的 `PASS/FAIL/SKIP/INFO` 宏、共享 `main.c`（`run_tests()` 入口 + 汇总打印）、"新二进制放 `/tests/` 即被自动发现"的约定全部不变。存量用例零迁移成本。

**Phase 3（测试用例 Rust 化）**：

* 新增 `no_std` Rust 测试框架 crate（`testcases/rust/`），通过 `build-std` + 裸机目标编译，由 initrd 携带；
* auditor 的标记协议对 C/Rust 用例一视同仁（同一串口协议，同一套 PASS/FAIL 宏语义）；
* 存量 C 用例按维护优先级渐进迁移，CMake 路径与 Rust 路径在 overture 中并存编译。

---

## 5. CI/CD 流水线集成

GitHub Actions 三架构矩阵 + KVM 加速：

```yaml
name: virtuoso-ci

on:
  push:
    branches: [ "main" ]
  pull_request:
    branches: [ "main" ]

jobs:
  kernel-e2e:
    name: E2E (${{ matrix.arch }})
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        arch: [ arm64, x86_64, riscv64 ]

    steps:
      - name: Checkout Code
        uses: actions/checkout@v4

      - name: Setup Rust Toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cargo Cache
        uses: Swatinem/rust-cache@v2

      - name: Install QEMU & Cross Toolchains
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            qemu-system-arm qemu-system-x86 qemu-system-misc \
            gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu cpio

      - name: Configure KVM Acceleration
        run: |
          echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666"' | sudo tee /etc/udev/rules.d/99-kvm.rules
          sudo udevadm trigger --name-match=kvm || true

      - name: Build Kernel
        run: |
          make ARCH=${{ matrix.arch == 'arm64' && 'arm64' || matrix.arch }} defconfig
          make -j"$(nproc)"

      - name: Virtuoso Verify
        run: cargo xtask verify --arch ${{ matrix.arch }}

      - name: Execute E2E Suite
        env:
          RUST_BACKTRACE: 1
        run: cargo xtask test --arch ${{ matrix.arch }} --timeout 600

      - name: Upload Serial Log & Events
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: virtuoso-${{ matrix.arch }}
          path: |
            target/serial.log
            target/events.jsonl
```

> 自建 runner（openEuler 宿主机）替代 GitHub runner 时，将安装步骤替换为 `dnf install qemu-system-aarch64 qemu-system-x86_64 qemu-system-riscv64`，其余流程不变；NUMA/大内存用例建议绑定带 KVM 与 NUMA 拓扑的自建 runner。

---

## 6. 迁移与演进路线图

```text
  Phase 1: 基座与对等               Phase 2: Rust 接管                Phase 3: AI 与全栈现代化
  ┌────────────────────────┐       ┌────────────────────────┐       ┌────────────────────────┐
  │ • xtask 引入,别名就绪   │       │ • overture 接管构建     │       │ • no_std Rust 测试用例  │
  │ • 8 个 make target 全部 │  ==>  │ • ensemble 全架构 DSL   │  ==>  │ • Firecracker microVM  │
  │   parity 映射(包装 sh)  │       │ • auditor 标记协议 v1   │       │ • triage 深化:根因聚类  │
  │ • .env 兼容读取         │       │ • coda RAII 全面接管    │       │ • 废弃全部 Makefile/sh  │
  └────────────────────────┘       └────────────────────────┘       └────────────────────────┘
```

**Phase 1 — 基座与对等（不改行为）**

* 建立 Workspace 与 `cargo xtask` 别名；8 个 target 以子进程方式包装现有 `verify.sh / build-initrd.sh / run-qemu.sh`，逐个验证 parity（退出码、标记输出、耗时）；
* 引入类型化配置读取层（`.env` 兼容），`verify` 先行输出配置诊断；
* 交付验收：现有 CI 脚本把 `make` 换成 `cargo xtask` 后行为完全一致。

**Phase 2 — Rust 接管（行为等价替换）**

* overture 接管 C 用例编译与 initrd 组装（含 `modules.conf` 语义）；
* ensemble 接管三架构启动（KVM/GDB/NVMe/VFIO/NUMA 全形态）；
* auditor 接管串口断言（协议 v1 冻结不变）并产出 `events.jsonl`；
* coda 全面接管进程组与挂载治理，删除 Makefile 的 PID 文件 hack；
* 废弃 `Makefile` 与三个 shell 脚本，`make qemu-test` 保留一行转发壳以防旧习惯。

**Phase 3 — AI 与全栈现代化**

* 新增测试用例默认 `no_std` Rust 编写，存量 C 用例渐进迁移；
* Firecracker 接入 ensemble 作为第二后端（microVM 池，缩短 boot 尾延迟）；
* AI triage 深化：跨 run 指纹聚类、补丁↔测试映射、最小复现生成。

---

## 7. 兼容性承诺

| 维度 | 承诺 |
|---|---|
| 串口标记协议 | v1 冻结：`[PASS]/[FAIL]/[SKIP]/[INFO]`、`Test Results: N/M`、`TEST_COMPLETE` 语义不变 |
| 退出码 | `0` 成功；非 `0` 失败（保留脚本真实退出码）；`124`（含 137 归一）超时。注意：GNU make 会把脚本失败折叠为其自身退出码 2 并吞掉 `exit 124`，cargo xtask 保留原始码，对 CI 的三态判定严格更优 |
| 配置 | Phase 1–2 兼容 `.env` 全部变量与语义（含 `QEMU_TIMEOUT=0` 拒绝、`KERNEL_PATH` 上级目录自动探测） |
| 测试用例 | 共享 `main.c`/`run_tests()` 约定与 `/tests/` 自动发现机制不变；`-static` 约束不变 |
| 目录 | 内核树内目录由 `qemu-e2e/` 更名 `virtuoso/`，支持旧名软链过渡一个版本 |
| AI | `kernel-virtuoso` skill 只依赖标记协议 v1 与 `events.jsonl`，不依赖 harness 内部实现 |

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

## 附录 B：决策记录

* **为什么机器类型锚定命名**：qemu-e2e 的 arm64/riscv64 machine type 即 `virt`，Virtuoso 以此为名，虚拟化属性内嵌于词根；
* **为什么冻结协议而非升级协议**：标记协议是 CI、AI skill、人眼三方的事实接口，升级收益小于迁移成本；如需 v2（结构化行内 JSON），在 events.jsonl 侧演进，串口文本保持 v1；
* **为什么 Phase 1 先包装后重写**：8 个 target 的 parity 验证是后续重写的行为基线，先锁定基线可让 Phase 2 的每一步替换都有对照测试。
* **退出码以 xtask 为准（Phase 1 parity 实测结论）**：`make` 将脚本失败统一折叠为退出码 2（含 `exit 124` 被吞），README 声明的"超时 124"在 make 路径下实际不可达；xtask 保留脚本真实退出码与 124 超时码，`cargo xtask parity <target>` 以三态判定（严格相等 / CI 等价双非零 / 失败）固化该结论，`--strict` 可强制严格对照。
* **BusyBox 供给走 CI 预编译 Release（随 Phase 1 落地）**：原实现本地源码编译且不分架构，交叉测试（arm64 宿主测 x86_64 内核）会缓存错误架构的 BusyBox；改为 `busybox-release` workflow 预编译发布 Release，`fetch-busybox.sh` 按架构下载（ELF 魔数校验）。供给链四层：显式 URL（`BUSYBOX_DL_URL`）→ `gh release download`（自带认证，私有仓库可用）→ 直链 wget（公开仓库）→ 源码归档兜底（busybox.net tarball → GitHub mirror）。仓库需公开 + 容器化两个实战教训：**交叉编译必须显式安装 `libc6-dev-<arch>-cross`**（仅 Recommends，`--no-install-recommends` 会漏装，`include_next` 会掉进宿主 `/usr/include`）；busybox 1.36.1 的 tc applet 无法在内核头文件 ≥ 6.8 下编译——构建环境已钉死 `ubuntu:22.04` 容器，同时发布改用 REST API（单容器 job，无 node 系 action 依赖）。
* **Phase 2 落地记录（2026-09）**：四 crate 接管完成 ——
  - **auditor**：标记协议 v1 解析 + `judge` 对账（`Verdict` 八态，含 panic-假通过防护：`-no-reboot` 下 panic 使 QEMU exit 0，以标记协议对账识破），11 个单测；
  - **ensemble**：`QemuInvocation` DSL + `Arch` 矩阵唯一事实源（从 xtask/config 迁入）。argv 与 run-qemu.sh **逐字对齐**（实测：`QEMU=echo` 双向 diff，shell/test 两路径 IDENTICAL）。勘误：脚本实际恒用 `virt` machine（q35 从未生效），DSL 以脚本真实行为为准；
  - **overture**：BusyBox 四层供给（ELF 魔数校验）、modules.conf 语义、C 用例编译、cpio newc + `du -sm+2` ext4 组装、verify 十一项检查、`InitHook`（rootfs 内 `/init-hooks.sh`，init 侧守卫 source）。实测新旧构建产物内容 diff：initramfs 433 项 IDENTICAL、rootfs 顶层与模块集 MATCH；
  - **coda**：`ProcessGroupGuard` RAII（Drop/超时/Ctrl-C 三路径收割，pgid=0 惰性防自杀），替代 Makefile PID 文件 hack。（重构清理：删除无人调用的 terminate/grace 机制，收割统一为 KILL 路径。）
  - **xtask**：test/`matrix`（三架构串行 + 总表）走新链路；`virtuoso.toml` 覆盖层（未知键解析期报错）；verify Rust 化。分层：`runs.rs` 承载运行工件与 triage/runs/replay 呈现，verdict.json schema 由 `report_json` 单点定义，`tasks.rs` 只保留编排。（分层：`runs.rs` 承载运行工件与 triage/runs/replay 呈现，verdict.json schema 由 `report_json` 单点定义；`tasks.rs` 只保留编排。）Makefile 全部 target 改为 `cargo xtask` 转发壳；`infra/*.sh` 保留为基线参考。实机验证：arm64/TCG boot → `verdict: passed`；`make qemu-test QEMU_TIMEOUT=0` 拒绝语义保留。CI：`virtuoso-ci.yml`（build/clippy/test + 自建 runner 手动 E2E）。
* **Phase 3 落地记录（2026-09）**：AI 与全栈现代化三项落地 ——
  - **no_std Rust 测试框架（§4 测试用例 Rust 化）**：`infra/testcases/rust/` 独立 workspace
    （主 workspace `exclude`），`testfw` 框架 crate + `test-rs-example` 骨架。`PASS/FAIL/SKIP/INFO`
    宏与 C `test_common.h` 一一对齐，`run_and_exit` 对齐共享 `main.c` 语义（失败 exit 1，
    `Test Results`/`TEST_COMPLETE` 仍由 init 汇编）——协议 v1 对 C/Rust 一视同仁，init 零改动。
    **构建决策偏差**：原案 build-std + 裸机目标需 nightly 与 rust-std 下载（实测环境镜像超时不可得），
    改为 **stable + 宿主 rust-std + `#![no_std] #![no_main]` 自定义 `_start`**：write/exit_group
    裸 syscall（write=64/1/64，exit_group=94/231/94），freestanding `memcpy/memmove/memset/memcmp`
    补齐（linux-gnu 的 compiler_builtins 默认留给 libc），`-static -nostdlib -nostartfiles -no-pie`
    冻结在 `rust/.cargo/config.toml`（产物为无 libc 纯静态 ELF）。交叉架构在宿主缺对应 rust-std 时
    显式 WARN 跳过（`overture::testcase::install_rust`，降级非掩盖——构建失败仍然 bail）。
    实机：arm64/TCG，C+Rust 双用例 `Test Results: 2/2 passed` → `verdict: passed`；
  - **encore（§3.6/3.7 AI triage 深化）**：语义收敛在最小摘要 `RunSummary`（verdict.json schema
    演进不外溢）；失败指纹 = verdict 类 + 归一化证据（剥离内核时间戳、数字折叠为 `N`；panic/oops
    优先 → timeout → 失败测试集），`cluster`/`flaky_tests`（flaky 判定只信 passed/failed 的 run）/
    `suggest_tests`（子系统路径前缀 → 最小测试集，缺省表 DEFAULT_RULES）。xtask 新命令：
    `cargo xtask cluster [--json]`、`cargo xtask suggest [--diff <f>] [--json]`（缺省对 KERNEL_PATH
    做 git diff）、`cargo xtask test --replay-until-fail N`（返场，首个非 passed 即停）；
  - **Firecracker 第二后端（ensemble::firecracker）**：config-file JSON（v1 API 冻结字段）+
    API 逐 PUT 序列 + `spawn`（独立进程组，串口走 stdout，与 QEMU 同收割约定）。
    `Backend` 枚举：CLI `--backend` > `BACKEND` 配置（virtuoso.toml `backend` 键）> 缺省 qemu；
    `verify --backend firecracker` 输出五项 preflight。硬约束 preflight 逐项给出可操作诊断：
    仅 x86_64/aarch64、KVM 必需、aarch64 内核必须 ELF、无 initramfs 引导要求
    virtio/virtio-blk/ext4/串口 `=y`。本机（无 KVM/binary）preflight 如实拒绝；
    实机 microVM 引导待具备 KVM 的自建 runner 验证；
  - **kernel-virtuoso skill（§3.7）**：`skills/kernel-virtuoso/SKILL.md` —— 数据接口集成
    （events.jsonl 唯一结构化事实源、分诊可回溯、AI 只读分析、脚手架生成、补丁映射、
    flaky 返场）；`cargo xtask skill install` 同时装 kernel-dev 与 kernel-virtuoso。
* **Phase 3 分层重构：角色名词命名 + 依赖分层（2026-09）**。动机：音乐隐喻名
  （overture/ensemble/auditor/coda/encore）不查文档猜不出职责；且存在层向颠倒
  （builder 前身 overture 依赖 launcher 前身 ensemble 只为用 `Arch`）、verdict.json
  schema 困在 CLI 层（tracker 前身 encore 需手工投影）、退出码语义三处重复、
  xtask 单体近 800 行等问题。落地：
  - **命名**：crate 全部改为角色名词 —— `common`（基础层，零依赖）、`builder`、
    `launcher`、`judge`、`guardian`、`tracker`；xtask 保留（cargo 社区约定名）。
  - **新基础层 common**：`Arch` 矩阵唯一事实来源移入 common::arch（launcher
    re-export 保持 API），`which`/ELF/可执行位/内存量解析/UTC 时间/人类可读大小
    收敛于此；builder→launcher 反向依赖消除。
  - **schema 归位**：`VerdictReport` + `RunMeta` 从 xtask/runs.rs 移入
    `judge::report`（构造 `VerdictReport::build`）；`judge::exit` 成为退出码语义
    唯一表（0/124/137→124/130；normalize + semantics）；tracker 通过
    `From<VerdictReport> for RunSummary` 闭环，diff→路径提取下沉 tracker。
  - **监管收敛**：ACTIVE_PGID 全局 + Ctrl-C 守护 + 看门狗从 xtask 移入
    `guardian::registry`；`Supervised`（登记+收割守卫组合句柄）与 launcher 的
    `spawn_supervised` 消除三处 spawn 样板。
  - **预检下沉**：firecracker 硬校验 `preflight` 与诊断视图 `preflight_checks`
    下沉 launcher::firecracker（xtask 两处重复实现合一）；verify 的模块存在性
    输入收集下沉 builder::verify::module_presence。
  - **xtask 拆分**：tasks.rs(776) → cli/{mod,verify,build,vm,parity,diagnostics}；
    runs.rs(809) → runs/{rundir,render}；config 只管解析，诊断呈现在 cli/diagnostics。
  - **不变量**：标记协议 v1、退出码 0/124/137、argv 逐字对齐、.env 语义、静态链接
    约束全部未动；verdict.json/events.jsonl 序列化输出逐字节一致（schema 仅换定义位置）。
