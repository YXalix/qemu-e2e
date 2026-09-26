# 两段式引导

> **面向未来维护者（包括 AI）的完整参考**：两阶段启动的架构、VM 内执行流程、
> 模块双清单规则、已知内核怪癖，以及"常见的修改应该动哪个文件"。
>
> 读前提：本文假设你知道 QEMU `-kernel`/`-initrd`/`-drive` 是什么。
> 镜像怎么来的见[供给与构建流水线](pipeline.md)。

## 1. 两阶段启动

```
qemu-system-<arch>
  ├─ -kernel  Image                              被测内核
  ├─ -initrd  initrd.img      ── 阶段 1 ──       极简 initramfs（内存盘）
  │            PID 1 = /init (= infra/init-initramfs)
  │            ① mount proc/sys/dev（含怪癖兜底，见 §4）
  │            ② insmod modules-boot.conf 里全部 .ko（仅"够到根盘"所需）
  │            ③ mount root=/dev/vda
  │            ④ exec switch_root /mnt /init
  │
  └─ -drive  rootfs.img       ── 阶段 2 ──       ext4 系统盘（virtio → /dev/vda）
               PID 1 = /init (= infra/init，测试 init)
               ① 幂等重挂 proc/sys/dev（switch_root 后多数已是挂载态）
               ② insmod modules.conf 里全部 .ko（组件 require 并集生成，/lib/modules/）
               ③ 交互 shell（默认）或 auto_test：跑 /tests/* → poweroff
```

**为什么是两阶段**：被测内核（openEuler）把 `CONFIG_VIRTIO_BLK`/`CONFIG_EXT4_FS`
编成模块（`=m`）。裸 ext4 root 会陷入"挂载驱动在盘上、盘需要驱动才能挂"的死锁。
initramfs 先在内存里加载驱动，是唯一出路——这也是 distro 的标准做法。

**职责边界（记住这条线，改错地方是最大风险）**：

| | initramfs（阶段 1） | rootfs（阶段 2） |
|---|---|---|
| 唯一使命 | 让 `root=` 可挂载 | 跑测试 / 提供交互环境 |
| 内容 | busybox + **boot 模块** + pivot init | busybox + **测试模块** + 测试 init + /tests（组子目录） + rootfs.d 增量 |
| 生命周期 | switch_root 后内存释放 | 持久 ext4，可 loop mount 随意改 |
| 变更频率 | 换内核/换 boot 模块时 | 加测试、改测试流程时 |

## 2. VM 内执行流程

### 阶段 1：`infra/init-initramfs`（busybox ash，POSIX 语法）

只做一件事：**让 `root=` 可挂载，然后 switch_root**。刻意不写任何测试逻辑。

```
mount proc/sys/devtmpfs/tmpfs          # 全部 2>/dev/null 容错
mknod console/null + mdev -s           # 怪癖兜底 A（见 §4），不可删
insmod modules-boot.conf 逐行          # best-effort，失败仅 WARN
解析 /proc/cmdline 的 root=（默认 /dev/vda）
轮询 [ -b $ROOT ] 最多 10s             # virtio 异步 probe 有延迟
[ ! -b ] 时从 /proc/partitions mknod   # 怪癖兜底 B（见 §4），不可删
mount -o rw $ROOT /mnt || 应急 shell
exec switch_root /mnt /init            # busybox 会把 /proc /sys /dev 挂载点搬进新根
```

### 阶段 2：`infra/init`（测试 init，进入 rootfs 后的 PID 1）

- `mount_filesystems`：重挂 proc/sys/dev/devpts/shm——**必须幂等**
  （switch_root 后大多已是挂载态，报错要吞掉）；同时保证 rootfs 被直启
  （root 驱动 =y 的内核、无 initramfs）时也能自举；
- `load_modules`：从 rootfs 的 `/lib/modules/modules.conf` insmod 测试模块，
  行格式 `<module> [key=val ...]`（参数原样传给 insmod）；
- 无 cmdline `auto_test` → `setsid cttyhack /bin/sh` 交互 shell；
  有 → 依次执行 `/tests/*`，汇总 `Test Results: N/M` +
  `TEST_COMPLETE: ALL/SOME TESTS FAILED/PASSED`（CI 断言此标记）→ `poweroff -f`。

## 3. 模块双清单规则

| 清单 | 语义 | 去向 | 谁加载 | 来源 |
|---|---|---|---|---|
| `infra/modules-boot.conf` | 让 root= 可挂载的最小集（**冻结基础集，手工维护**） | initramfs `/lib/modules/` | `init-initramfs`，pivot 前 | 手写 conf 文件 |
| 组件 `stage = "boot"` 附加 | 某组件需要引导早期可用 | 同上（追加在基础集后） | 同上 | `virtuoso.toml` `[components.*] require` |
| 生成 `modules.conf` | 启用组件的一切模块需求 | rootfs.img `/lib/modules/` | `infra/init`，pivot 后 | 组件 require 并集（builder 生成） |

**判断标准：这个模块是不是"root 挂上之前就必须在内核里"？是 → 组件加
`stage = "boot"`，否 → 缺省 runtime。**require 条目 = conf 行
`"<module> [key=val ...]"`；并集按组件固定顺序去重保首个，条目顺序即
insmod 顺序（被依赖者在前）。缺 `.ko` 构建期报错。

组件机制总览见[组件机制](components.md)。

## 4. 已知内核怪癖与必要 workaround（不可删！）

针对 openEuler 内核 + 修改版 QEMU（含 UBIOS 日志）实测发现。删掉任何一个都会
回到 `cannot mount root=/dev/vda` 或 panic：

| # | 现象 | 根因 | 兜底（位置） |
|---|---|---|---|
| A | initramfs 里 `/dev` 是空的 | devtmpfs 挂上了但不自动填充 | `mknod console/null` + `mdev -s`（init-initramfs 开头） |
| B | `/sys/class/block/vda/dev` 内容为空 | sysfs dev 属性不输出 | 从 `/proc/partitions` 解析 major:minor 后 `mknod`（init-initramfs 中段） |
| C | 裸 ext4 root 直接 panic | `CONFIG_VIRTIO_BLK`/`CONFIG_EXT4_FS` = m | 两阶段架构本身（§1） |
| D | busybox 1.36.1 `tc` applet 编不过 | 内核头 ≥ 6.8 不兼容 | CI 里 `sed` 关掉 CONFIG_TC（workflow） |

## 5. 修改任务手册

**加一个测试模块**（最常见）
→ 在 `virtuoso.toml` 里给对应组件加/启用 `require`（注意依赖顺序，被依赖者
在前），`virtuoso build`。**不要**碰 `modules-boot.conf`——那会让
initrd.img 无谓变化。

**加测试用例** → 见[编写测试用例](../usage/writing-tests.md)；
改测试流程 → 改 `infra/init`。验证：`virtuoso build && virtuoso test --timeout 30`。

**加文件进 rootfs**（测试数据、脚本用例、环境文件）
→ 放仓库根 `rootfs.d/`，构建时整树增量并入 rootfs。**只增不覆盖**：
与组装产物同路径 = 构建期报错；`rootfs.d/tests/<组>/run-x.sh` 就是脚本
用例（按退出码判定）。详见[编写测试用例](../usage/writing-tests.md)。

**VM 内环境定制**（如预分配大页）
→ 通用定制点写 `init-hooks.sh` 片段（builder 注入 rootfs `/init-hooks.sh`，
init 侧守卫 source，位于 devtmpfs 挂载后、insmod/agent 拉起前）：

  ```sh
  mkdir -p /mnt/huge
  mount -t hugetlbfs nodev /mnt/huge
  echo 20 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
  ```

**改 pivot / 提前加载行为**
→ 改 `infra/init-initramfs`。守住两条：只做"够到 root="的事；
POSIX/busybox-ash 语法（本地校验：
`busybox sh -n infra/init-initramfs`）。改完重跑 §6 的验证手册。

**换 rootfs 发行版**（如 Alpine/debootstrap）
→ rootfs.img 就是普通 ext4。改 builder 的 rootfs 组装段，或在 VM 外
loop mount 后替换内容。两阶段启动链不需要任何改动。

**发新 busybox 版本** → Actions 手动触发 `busybox-release`（填版本号），
然后 `virtuoso.toml` 里 `[busybox] version = <ver>`。

**改 release rootfs 的 /init**
→ 原则上不要改（冻结不变量，见[供给与构建流水线](pipeline.md)）。确需修改：
编辑 workflow 里 printf 段落，重跑 workflow。**用 printf，不要用 heredoc**——
YAML 块标量的缩进剥离会让 `EOF` 终止符对不上（dash 的 `<<-` 只认 tab，
而 YAML 禁 tab），CI 会报 `syntax error: end of file unexpected`。

**改 shell 脚本通用规则**：GitHub Actions 的 `run:` 用 dash 执行——
`local a=1 b=$a` 同行互引会取到空值（必须分行）、不支持花括号展开
(`{a,b}`)、不支持 `${@:2}`（用 `shift`）。VM 内脚本（`init`、
`init-initramfs`）以 busybox ash 为准，同样按 POSIX 写，改完先做
`busybox sh -n` 语法校验。

## 6. 验证手册（改动后必跑）

```bash
# 0. 语法
busybox sh -n infra/init-initramfs && busybox sh -n infra/init

# 1. 构建
virtuoso build

# 2. 交互模式：应看到 [initramfs] 模块日志 → root 挂载 → 测试 init 横幅 → ~ # shell
virtuoso shell

# 3. 自动测试：应输出 TEST_COMPLETE: ALL TESTS PASSED 且干净关机（exit 0）
virtuoso test --timeout 30
```

需要脱离 virtuoso 直接驱 QEMU 排查时（等价形态）：

```bash
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 1G -nographic -no-reboot \
  -kernel $KERNEL_PATH/arch/arm64/boot/Image -initrd target/artifacts/initrd.img \
  -drive file=target/artifacts/rootfs.img,format=raw,if=virtio \
  -append "console=ttyAMA0 root=/dev/vda rw init=/init loglevel=3"
```
