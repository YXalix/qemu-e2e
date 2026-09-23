# 配置参考

`virtuoso.toml` 是唯一配置面。仓库根的模板即缺省常规启动配置，可选能力全部
以注释形式在场，取消注释即启用。

- **优先级**：同名标量键以**进程环境变量**为准（env 覆盖 toml，CI / 命令行
  临时改参不动文件）；持久配置只写 toml。
- **严格解析**：未知键 / 非法类型解析期报错，不静默忽略。
- 诊断呈现：`virtuoso verify`（全量）或 `virtuoso doctor`（一屏）。

## 全局键

| 键 | 环境变量 | 说明 |
|---|---|---|
| `arch` | `ARCH` | `x86_64` \| `arm64`（缺省）\| `riscv64` |
| `timeout_secs` | `TIMEOUT_SECS` | 墙钟超时秒数；0 一律拒绝 |
| `smp` | `SMP` | vCPU 总数；多节点 NUMA 时必须被节点数整除（解析期校验） |
| `auto_test` | `AUTO_TEST` | true = 跑完 `/tests/` 自动关机；false = 落入交互 shell |
| `kernel_path` | `KERNEL_PATH` | 内核树路径（装置在树内时可自动探测） |
| `kernel_image` | `KERNEL_IMAGE` | 内核镜像覆盖（缺省 = 内核树内 arch 对应镜像） |
| `qemu` | `QEMU` | QEMU 二进制覆盖（`QEMU=echo` 可打印 argv 对照） |
| `qemu_opts` | — | 透传兜底参数数组（如 ivshmem） |

## `[components.*]` 组件段

每个组件段的公共字段（`enabled` / `require` / `stage`）与逐组件的专属键见
[组件机制](../components/overview.md)。

## `[busybox]` 段

| 键 | 说明 |
|---|---|
| `version` | BusyBox 版本（如 `"1.36.1"`） |
| `release_repo` | 预编译 Release 仓库（缺省扫描 git remotes 找 github.com） |
| `dl_url` | 显式下载 URL（供给链第一优先级） |
| `force_source_build` | true = 跳过下载，直接源码编译 |

供给链详解见[两段式引导与构建流水线](../internals/boot-pipeline.md)。

## 架构矩阵

| arch | QEMU 二进制 | 内核镜像 | 控制台 | 机型 |
|---|---|---|---|---|
| `arm64`（缺省） | `qemu-system-aarch64` | `arch/arm64/boot/Image` | `ttyAMA0` | `virt` |
| `x86_64` | `qemu-system-x86_64` | `arch/x86/boot/bzImage` | `ttyS0` | `q35` |
| `riscv64` | `qemu-system-riscv64` | `arch/riscv/boot/Image` | `ttyS0` | `virt` |

交叉组合随意（如 arm64 宿主跑 `--arch x86_64`），目标与宿主不同构时
doctor / verify 会告警，启动自动回退 TCG。
