# 使用指南

日常操作手册：从零上手、配置、写用例、加载模块、调试、组件与运行工件。
两段式引导的逐行解读见 [Initramfs 与 Rootfs 构建指南](initramfs-rootfs-guide.md)，
故障速查见 [Troubleshooting](troubleshooting.md)，总体架构与冻结契约见
[架构与设计](virtuoso-design.md)。

---

## 1. 快速开始

### 1.1 前置条件

- 宿主工具：`rust`（stable）、`gcc` / `cmake`、`wget` / `cpio` / `gzip`、`qemu-system-*`
- 内核源码树（本装置设计为放进内核树内运行，如 `kernel/virtuoso/`）
- openEuler / Fedora：`sudo dnf install -y gcc make cmake wget cpio gzip qemu-system-aarch64 qemu-img`
- Debian / Ubuntu：`sudo apt install -y gcc make cmake wget cpio gzip qemu-system-arm qemu-utils`

### 1.2 构建内核

```bash
git clone https://gitcode.com/openeuler/kernel.git && cd kernel
cp arch/arm64/configs/openeuler_defconfig .config   # 或自己的 .config
make -j"$(nproc)"
make modules -j"$(nproc)"        # 只要有任何模块是 =m
```

装置默认放在内核树内（自动探测）；放别处时在 `virtuoso.toml` 设 `kernel_path`。

### 1.3 验证与首跑

```bash
cargo install --path xtask    # 规范二进制 virtuoso 装入 PATH（一次）；工作区内 cargo xtask / cargo v 别名等价
virtuoso doctor               # 一屏体检：✓/✗ 组件行，最快确认接线
virtuoso test --timeout 60    # 构建 → 启动 → 判定 → 工件落盘
virtuoso triage               # 分诊最近一次运行
```

**AI 的标准验证循环：`doctor → test → triage`。判定以 triage 的 verdict 为准**：
`verdict: passed` 才算通过。退出码只是接口契约（0=通过、124=超时、其余=失败）——
`-no-reboot` 下内核 panic 会让 QEMU 以 exit 0 退出，只看退出码会假通过；
`exit 0` 但 `verdict: incomplete` = 标记协议没走完，同样按失败处理。

Verdict 全集（`judge::Verdict`）：`passed` / `failed` / `timeout` / `panic` /
`incomplete` / `interrupted` / `build_failed`。

`doctor` 与 `verify` 共用同一检查引擎（`builder::verify`），只是呈现繁简之别：
doctor 把全部检查归并为 ✓/✗/! 组件行（Config / Toolchain / Kernel / QEMU /
Modules / Artifacts，firecracker 后端追加 Firecracker 组）；`virtuoso verify`
则附全量清单与类型化配置诊断。doctor 报 ✗ 时用 verify 看细节；两者同参
（`--arch` / `--backend`，doctor 另有 `--json`）。

---

## 2. 配置：`virtuoso.toml`

唯一配置面。标量键优先级：进程环境变量 > `virtuoso.toml`（同名键 env 覆盖
toml，临时改参不动文件，持久配置写 toml）；未知键 / 非法类型解析期报错。
仓库根的 `virtuoso.toml` 模板即
缺省常规启动配置，可选能力全部以注释形式在场，取消注释即启用。

### 2.1 全局键

| 键 | 说明 |
|---|---|
| `arch` | `x86_64` \| `arm64`（缺省）\| `riscv64` |
| `timeout_secs` | 墙钟超时秒数；0 一律拒绝 |
| `smp` | vCPU 总数；多节点 NUMA 时必须被节点数整除（解析期校验） |
| `backend` | `qemu`（缺省）\| `firecracker`（microVM，x86_64/aarch64 + KVM） |
| `auto_test` | true = 跑完 `/tests/` 自动关机；false = 落入交互 shell |
| `kernel_path` | 内核树路径（装置在树内时可自动探测） |
| `kernel_image` | 内核镜像覆盖（firecracker aarch64 需 ELF 时用） |
| `qemu` | QEMU 二进制覆盖 |
| `qemu_opts` | 透传兜底参数数组（如 ivshmem） |
| `firecracker_bin` | firecracker 可执行文件路径 |

### 2.2 组件（VM 能力）

每个 `[components.*]` 段支持：

- `enabled`：开关；**段缺省 = 各组件自己的缺省**（tools_disk 缺省启用，其余缺省关闭）
- `require`：KO 依赖，条目 = conf 行 `"<module> [key=val ...]"`；builder 把启用
  组件的 require 并集（schema 固定顺序 tools_disk→agent→vfio→numa→pmem，去重保首）
  生成 rootfs `/lib/modules/modules.conf`
- `stage`：`boot`（root 挂载前就要）\| `runtime`（缺省）；boot 条目追加到
  initramfs 的 modules-boot.conf 冻结基础集之后

| 组件 | 作用 | 关键键 |
|---|---|---|
| `tools_disk` | tools.img 数据盘 → `/dev/vdb` 挂 `/tools`（VM 内工具，musl 静态） | 缺省启用 |
| `agent` | AI probe 通道（virtio-serial + guest 侧 virtuoso-agent） | `require = ["virtio_console"]` |
| `vfio` | PCI 直通，逐条生成 `-device vfio-pci,host=<bdf>` | `devices = ["0000:01:00.0"]`（宿主需开 IOMMU） |
| `numa` | 多节点拓扑（每节点一个 socket） | `nodes`、`memory_per_node` |
| `pmem` | 持久内存（DT 途径 → `/dev/pmem0` + DAX；从 guest RAM 顶部挖出） | `size`（须小于总内存）、`require`（内核 =m 时声明） |
| `[busybox]` | BusyBox 版本（如 `"1.36.1"`） | 全局段，非组件 |

firecracker 后端不支持 agent 通道与 pmem（启用时 WARN 忽略）。`virtuoso probe`
恒开 agent 通道，不依赖组件开关。

### 2.3 架构矩阵

| arch | QEMU 二进制 | 内核镜像 | 控制台 | 机型 |
|---|---|---|---|---|
| `arm64`（缺省） | `qemu-system-aarch64` | `arch/arm64/boot/Image` | `ttyAMA0` | `virt` |
| `x86_64` | `qemu-system-x86_64` | `arch/x86/boot/bzImage` | `ttyS0` | `q35` |
| `riscv64` | `qemu-system-riscv64` | `arch/riscv/boot/Image` | `ttyS0` | `virt` |

交叉组合随意（如 arm64 宿主跑 `--arch x86_64`），目标与宿主不同构时 doctor / verify 会告警。

---

## 3. 测试用例

`init` 自动发现 rootfs `/tests/` 下的所有二进制，新增用例零接线。
测试必须静态链接（`-static`，VM 内无动态加载器），禁止用 `|| true` 掩盖失败。

### 3.1 C 用例

1. 新建 `infra/testcases/src/test_<name>.c`：

```c
#include "test_common.h"

static void test_my_feature(void)
{
    printf("\nTest: my feature does the thing\n");

    /* 通过 syscall / ioctl / /proc / /sys / /dev 驱动被测内核 */
    if (/* 期望条件 */)
        PASS("the thing happened");
    else
        FAIL("expected X, got Y");
}

void run_tests(void)
{
    test_my_feature();
}
```

宏：`PASS` / `FAIL` / `SKIP` / `INFO`；不要写自己的 `main()`（共享的会被链入）。

2. 在 `infra/testcases/CMakeLists.txt` 注册（构建为 `-static -O2 -Wall`）：

```cmake
add_executable(test-<name>
    src/main.c
    src/test_common.c
    src/test_<name>.c
)
set_target_properties(test-<name> PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
```

3. 重建并运行：

```bash
virtuoso build && virtuoso test --timeout 30
```

### 3.2 Rust 用例（no_std）

`infra/testcases/rust/` 是独立 workspace（不进主 workspace 依赖图）：
`framework/` 是 `testfw` no_std 框架，用例 crate 裸 syscall、静态 ELF，
构建产物同样拷入 `/tests/` 自动发现。骨架参考 `test-rs-example`。

### 3.3 串口标记协议 v1（冻结）

判定协议不可改动（改文本等于破坏所有下游解析）：

- `[PASS] / [FAIL] / [SKIP] / [INFO]` — 单断言行
- `Test Results: N/M passed` — 单二进制汇总
- `TEST_COMPLETE: ALL TESTS PASSED | SOME TESTS FAILED` — 全局终止标记
- 退出码 `124` — harness 墙钟超时（内核 hang 或失控循环）

---

## 4. 内核模块

模块供给**由组件生成**，不手写清单：

- `infra/modules-boot.conf`（冻结基础集：virtio + ext4 及依赖）进 initramfs，
  由 `init-initramfs` 在 pivot 前 insmod；组件条目 `stage = "boot"` 追加其后。
- 其余来自启用组件的 `require` 并集，builder 写入 rootfs
  `/lib/modules/modules.conf`，测试 init 在 pivot 后加载。

规则：

- **顺序敏感**：按清单顺序 insmod（无自动依赖解析），依赖在前。
- **模块必须已构建**：`kernel_path` 下找不到对应 `.ko` 直接构建报错。
- **迭代中的模块优先 `=m` 而非 `=y`**：免内核重建，回路更快。

---

## 5. 调试

### 5.1 交互 shell

```bash
virtuoso shell            # TCG（跨架构适用）
virtuoso shell --kvm      # 原生加速（宿主 = 目标架构时）
```

退出：`Ctrl-A` 然后 `x`。

### 5.2 GDB 源码级内核调试

内核开 `CONFIG_DEBUG_INFO=y`（建议加 `CONFIG_DEBUG_INFO_DWARF5=y`、`CONFIG_GDB_SCRIPTS=y`）。

```bash
virtuoso debug            # 终端 1：挂起启动，监听 :1234
cd "$KERNEL_PATH"            # 终端 2：
gdb-multiarch vmlinux -ex 'target remote :1234'
```

### 5.3 AI probe（virtio-serial 通道）

```bash
virtuoso probe --cmd 'uname -a' --cmd 'dmesg | tail'
virtuoso probe --cmd-file cmds.txt --json    # 机器可读事件流
```

经 guest 侧 `tools/virtuoso-agent`（JSON 行协议）下发命令批，事件流写
run 目录的 `agent-events.jsonl`；`probe` 恒开 agent 通道。

### 5.4 Firecracker 后端

```bash
virtuoso doctor --backend firecracker    # 一屏体检（含 microVM preflight 组）
virtuoso verify --backend firecracker    # 全量清单 + microVM preflight
virtuoso test --timeout 60 --backend firecracker
```

x86_64 / aarch64 + KVM；不支持 agent 通道与 pmem（WARN 忽略）。

---

## 6. 运行工件与跨 run 语义

每次 `test` / `matrix` / `probe` 落盘 `target/runs/<unix_ms>-<arch>/`
（保留最近 20 次）：`serial.log`、`qemu-stderr.log`、`build.log`、
`events.jsonl`、`verdict.json`（probe 无 verdict，另有 `agent-events.jsonl`）。

```bash
virtuoso triage [--run <id>] [--json]
virtuoso runs [--json]              # 历史运行
virtuoso replay --log <f>           # 离线标记协议断言（不启动 QEMU）
virtuoso matrix [--arch a]          # 多架构矩阵（缺省三架构，串行）
virtuoso test --replay-until-fail 5 # flaky 返场：首个非 passed 即停
virtuoso cluster [--json]           # 跨 run 失败指纹聚类 + flaky 清单
virtuoso suggest [--diff f]         # git diff → 推荐最小测试集
```

`triage` / `runs` / `cluster` / `suggest` / `replay` 都支持 `--json`，可直接进管道。

---

## 7. VM 内定制

VM 内资产（init、testcases、tools）都在 `infra/`，git 跟踪、构建时注入镜像：

- **测试 init（`infra/init`）**：PID 1 脚本。通用定制点写 `init-hooks.sh`
  片段（builder 注入 rootfs `/init-hooks.sh`，init 侧守卫 source，PATH 注入后）；
  例如预分配大页：

  ```sh
  mkdir -p /mnt/huge
  mount -t hugetlbfs nodev /mnt/huge
  echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
  ```

- **VM 内工具（`infra/tools/`）**：std Rust + musl 静态，装 tools.img 的
  `/bin/`（guest 内挂 `/tools` 注入 PATH），不进 `/tests/`、不参与判定。

改完 `infra/` 后重建：`virtuoso build`。
哪些文件改了会进哪个镜像，见
[Initramfs 与 Rootfs 构建指南](initramfs-rootfs-guide.md) 的文件索引与怪癖表
（已知怪癖**不要"修复"**——都是 openEuler 内核的实测行为）。

---

## 8. AI skill 集成

```bash
virtuoso skill install      # kernel-dev + kernel-virtuoso skill 装入内核树
virtuoso skill uninstall
```

`kernel-dev` 教 AI 驱动测试回路 / 写用例 / 解析串口 / 分诊失败；
`kernel-virtuoso` 是 AI 数据接口集成（probe 通道）。装入后 AI 在内核树内自动发现。

---

## 9. 贡献约定

- VM 内 shell 代码保持 POSIX 兼容（`init` 跑在 BusyBox `sh`，不是 bash）。
- 新增前置条件 → 同步 `crates/builder/src/verify.rs`（doctor 的分组呈现自动跟随）。
- harness 暴露新行为 → 配一个对应测试用例。
- 用户可见行为变化 → 更新本文档与 `skills/kernel-dev/SKILL.md`。
- 冻结项不许动：标记协议 v1、test 退出码语义、`QemuInvocation::argv` 基线
  （详见 AGENTS.md「冻结的不变量」）。
