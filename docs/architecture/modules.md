# 核心模块设计

单包 crate 的内部模块设计。总体架构见[上一页](overview.md)；对外冻结的协议与
契约见[冻结契约](contracts.md)。

## 顶层：arch 与 util

零依赖的横切类型住在 `src/` 顶层两个文件：`arch.rs`（`Arch` 架构矩阵唯一
事实来源：QEMU 二进制 / 内核镜像路径 / console / machine / triple 系列；
`HostOs` 宿主平台分类同文件）与 `util.rs`（原 common 基础层摊平后的单文件
形态：fsutil 的 `which` 与 ELF 探测、内存量解析、UTC 时间、人类可读大小、
进度输出、终端颜色、shell 引用、中断退出码常量，分节承载）。领域模块与
cli 直接 `use crate::…`，无中间层。

## src/config.rs — 类型化配置

`virtuoso.toml` 是唯一配置面：全局键 + `[components.*]` 组件段 +
`[busybox]` 版本段。标量键优先级：进程环境变量 > `virtuoso.toml`
（同名键 env 覆盖 toml，临时改参不动文件）；未知键 / 非法类型解析期报错
（`deny_unknown_fields`，逐结构体显式声明而非 flatten）。单文件按关注点
分节：toml schema（结构与解析期防线）→ 全局标量访问器 → 组件访问器 →
BusyBox 供给配置 → ComponentPlan（并集分区）。

`ComponentPlan` 把启用组件的 require 并集（schema 固定顺序
tools_disk→agent→vfio→numa→pmem，按首 token 去重保首个）按 stage 分区：
`boot_extra` 追加到 initramfs 冻结基础集之后，`runtime` 生成 rootfs
`/lib/modules/modules.conf`。`virtuoso probe` 恒开 agent 通道（强制并入
virtio_console，不依赖组件开关）。

配置面全量见[配置参考](../guide/configuration.md)，组件机制见
[组件机制](../components/overview.md)。

## src/builder/ — 构建器

* **BusyBox 四层供给**：显式 URL（`BUSYBOX_DL_URL`）→ `gh release download`（`busybox-<ver>-linux-<arch>`，自带认证）→ 直链 wget（ELF 魔数校验）→ 源码归档兜底（busybox.net tarball → GitHub mirror）。产物按架构缓存于 `target/build/busybox/bin/`。
* **模块清单生成（modconf）**：boot = `modules-boot.conf` 冻结基础集 + 组件 `stage="boot"` 附加；runtime = 启用组件 require 并集（末尾并入 `[tests].require` 测例模块依赖）生成的 `modules.conf`。按清单顺序 insmod（依赖手工排序，缺 `.ko` 构建期报错）。
* **测试用例（C over Rust）**：`infra/testcases/` 独立 workspace，coda（std）为框架与统一入口（banner/panic hook/C 段调用/计数/退出码都在 `coda::run_and_exit`），C 测试体经用例 crate 的一行 build.rs（coda-scaffold 共享实现，cc crate）编入同一 musl 静态二进制（配方与 tools 同：零 rustflags + `--target <musl triple>`）；`c/` 为空时 coda-scaffold 编入空桩 `run_c_tests` 兜住符号。C 编译器由 `cross` 以 `CC_<TRIPLE>` 注入 zig cc 包装，交叉构建无 CC 接管时 coda-scaffold fail-fast（防 glibc/musl 混链）。宿主缺对应架构 rust-std 时显式 WARN 跳过（构建失败仍然报错）。新用例 = 复制 test-example 目录（members 走 `test-*` glob，零接线）。
* **tools.img**：`infra/tools/`（std Rust + musl 静态）装入 ext4 数据盘 `/bin/`（卷标 `tools`），rootfs 不装工具；工具供给缺失时不产出 tools.img、不注入挂载 hook（降级显式非掩盖）。
* **cpio + ext4 组装**：initramfs（cpio newc）+ rootfs/tools（ext4，尺寸 = `du -sm` + 2MB 余量）。
* **InitHook 注入点**：builder 生成 rootfs 内 `/init-hooks.sh`（init 侧守卫 source，位于 devtmpfs 挂载后、insmod/agent 拉起前），tools 盘挂载 + PATH 注入即走此通道；VM 内定制（如 hugetlbfs 预分配）写 hook 片段即可。
* **verify 检查引擎**：前置检查 + 类型化配置诊断（工具链 / 内核镜像 / QEMU / 模块 / BusyBox 缓存 / 组件状态）；doctor 与 verify 共用，两侧呈现自动跟随。

## src/launcher/ — 启动 DSL、进程治理与双后端

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
详见 [pmem 组件页](../components/pmem.md)。

### 进程治理（launcher::guardian）

进程治理与启动是一体的生命周期，作为 launcher 的 `guardian` 子模块承载：

* **`ProcessGroupGuard`（RAII）**：QEMU 进程组收割，Drop / 超时 / Ctrl-C 三路径统一 KILL；pgid=0 惰性登记防自杀。
* **注册表与看门狗**：活动进程组全局注册表 + Ctrl-C 守护（`install_ctrlc_guard`，cli 入口装载）+ 墙钟看门狗。`Supervised` 是"登记 + 收割守卫"组合句柄，与 `spawn_supervised` 消除 spawn 样板。

`virtuoso test` 中途被 Ctrl-C 打断时：收割 QEMU 进程组 → 落盘已产出的 run 工件 → 以 130 退出，宿主机不残留虚拟化进程。

## src/judge.rs — 判定引擎

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
`judge`（`EXIT_TIMEOUT` / `normalize`：0=通过、124=超时、其余=失败）。工件详解见
[运行工件与分诊](../guide/artifacts.md)。
