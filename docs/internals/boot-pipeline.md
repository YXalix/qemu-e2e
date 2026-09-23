# 两段式引导与构建流水线

> **面向未来维护者（包括 AI）的完整参考**：两阶段启动的架构、资产供给链、
> 构建流水线、VM 内执行流程，以及"常见的修改应该动哪个文件"。
>
> 读前提：本文假设你已经知道 QEMU `-kernel`/`-initrd`/`-drive` 是什么。
> 相关设计背景见[总体架构](../architecture/overview.md)。

## 1. 架构总览：两阶段启动

```
qemu-system-<arch>
  ├─ -kernel  Image                              被测内核
  ├─ -initrd  initrd.img      ── 阶段 1 ──       极简 initramfs（内存盘）
  │            PID 1 = /init (= infra/init-initramfs)
  │            ① mount proc/sys/dev（含怪癖兜底，见 §6）
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
| 内容 | busybox + **boot 模块** + pivot init | busybox + **测试模块** + 测试 init + /tests |
| 生命周期 | switch_root 后内存释放 | 持久 ext4，可 loop mount 随意改 |
| 变更频率 | 换内核/换 boot 模块时 | 加测试、改测试流程时 |

## 2. 资产供给链

### 2.1 GitHub Release（通用产物，刻意保持稳定）

由 `.github/workflows/busybox-release.yml` 生成，tag `busybox-v<version>`。
每架构两个资产：

| 资产 | 内容 |
|---|---|
| `busybox-<ver>-linux-<arch>` | 裸静态 busybox 二进制（defconfig + `CONFIG_STATIC=y`） |
| `rootfs-<ver>-linux-<arch>.cpio.gz` | 通用 busybox rootfs 目录树（cpio.gz 形式） |

**稳定性不变量（重要）**：

- release rootfs 的 `/init` 是**冻结在 workflow 里的 10 行脚本**（printf 拼接），
  不引用任何仓库脚本。→ 改仓库任何 `.sh`/`init*` 都**不需要**重发 release；
  资产内容只随 busybox 版本变化。
- release **不含**任何测试内容（无 `/tests`、无测试 init、无模块清单）。
  模块清单由组件 require 并集生成（§5），测试是本仓库的场景注入，由本地
  构建（§3）完成。
- rootfs 目录树：`bin/`(busybox + 全量 applet 符号链接) + `sbin usr/{bin,sbin}`
  + 空骨架目录（`proc sys dev tmp mnt etc/init.d var/run root lib`）
  + `/etc/passwd`、`/etc/group`（root）。
- CI 中三架构 applet 集合一致（同为 defconfig）。本地构建侧的 applet 符号
  链接由名单驱动（`infra/busybox/applets-<version>.txt`，名单即 x86_64
  `busybox --list` 输出的冻结版），**不执行 guest 二进制**——交叉组装
  （含 macOS 宿主）无需宿主同构；自定义版本/配置走 `$BUSYBOX_APPLETS_FILE`。

重新发布：Actions 页手动 `workflow_dispatch`（默认 1.36.1），或推 `busybox-v*` tag。
发布是幂等的：同名 asset 先删后传，release 已存在则复用。

### 2.2 BusyBox 二进制的本地供给链（builder::busybox，`virtuoso busybox`）

按序尝试，命中即缓存到 `target/build/busybox/bin/busybox-<arch>`：

```
0. 本地缓存  →  1. $BUSYBOX_DL_URL  →  2. gh release download（带认证，私有库可用）
→  3. 直链 wget/curl  →  4. 源码编译兜底（仅 Linux 宿主；交叉时告警）
```

发布仓库推导：`$BUSYBOX_RELEASE_REPO` > 扫描 git remotes 找 github.com 的那个。

### 2.3 cpio.gz → ext4 转换（并入 `virtuoso build`）

ext4 打包由 builder 完成（`crates/builder/src/image.rs::make_ext4`，
`mke2fs -d` + 自动尺寸）：先 cpio 解包成树，再按 `du -sm` + **2MB 余量**
生成镜像（绝不要拍固定尺寸——曾有过 64MB 空洞的教训，ext4 的元数据 +
journal 会把创建时声明的大小全部真实占满）。release 的 rootfs cpio.gz
如需单独转 ext4，解包后 loop mount 即可。

## 3. 本地构建流水线（builder，`virtuoso build`）

**输入**：`kernel_path`（内核源码树，找 .ko）、`arch`、`modules-boot.conf`
（冻结 boot 基础集）、`virtuoso.toml` 组件 require 并集（boot 附加 + runtime）、
`infra/init-initramfs`、`infra/init`、testcases/、tools/。
**输出**：`target/artifacts/initrd.img`（cpio.gz）+ `target/artifacts/rootfs.img`
（ext4，自动尺寸）+ `target/artifacts/tools.img`（ext4 卷标 `tools`，VM 内挂
`/tools`；tools 被跳过时不产出）；
暂存目录与 busybox 缓存在 `target/build/`。

流水线步骤（函数级）：

1. busybox 供给（§2.2）→ 拿到 per-arch 静态 busybox；
2. busybox 树组装 → busybox 二进制 + **applet 名单驱动的相对符号链接**
   （`builder::busybox::applet_names` 三级解析：`$BUSYBOX_APPLETS_FILE` →
   `infra/busybox/applets-<ver>.txt` → 宿主同构时 `--list` 现取）
   + 骨架目录 + passwd/group；
3. **initramfs 树** = busybox 树 + `init-initramfs` → `/init`
   + `modules-boot.conf` 冻结基础集 + 组件 `stage = "boot"` 附加条目
   （生成 conf 随行）→ **`builder::cpio` 原生打包**（newc + gzip，Rust 单路径，
   无 GNU cpio/wget 依赖；mtime 恒 0 + 路径排序 → 产物确定性可复现）；
4. **rootfs 树** = busybox 树 + `init`（测试 init）→ `/init`
   + 组件 require 并集生成 `modules.conf` → `/lib/modules/`
   + 构建 testcases（C/CMake + Rust no_std 静态二进制 → `/tests/`；
   非 Linux 宿主经 `builder::cross` 接 zig cc，Rust 恒 `--target <musl triple>`）
   + 生成 `/init-hooks.sh`（tools.img 挂载 hook，见下）
   → `mke2fs -d` 打成 ext4（尺寸 = `du -sm` + 2MB；macOS 上 mke2fs 探测
   brew keg 路径）；
5. **tools.img 树** = tools workspace 的 musl 静态产物 → `/bin`
   → `mke2fs -d` 打成 ext4（卷标 `tools`）。启动时作为第二个 virtio-blk
   （`/dev/vdb`）附加（`tools_disk_opt`：产物存在才附加），rootfs 的
   `/init-hooks.sh` 挂载到 `/tools` 并把 `/tools/bin` 注入 PATH——agent 与
   常驻工具随此盘走，rootfs 不装工具。

## 4. VM 内执行流程逐行说明

### 4.1 阶段 1：`infra/init-initramfs`（busybox ash，POSIX 语法）

只做一件事：**让 `root=` 可挂载，然后 switch_root**。刻意不写任何测试逻辑。

```
mount proc/sys/devtmpfs/tmpfs          # 全部 2>/dev/null 容错
mknod console/null + mdev -s           # 怪癖兜底 A（见 §6），不可删
insmod modules-boot.conf 逐行          # best-effort，失败仅 WARN
解析 /proc/cmdline 的 root=（默认 /dev/vda）
轮询 [ -b $ROOT ] 最多 10s             # virtio 异步 probe 有延迟
[ ! -b ] 时从 /proc/partitions mknod   # 怪癖兜底 B（见 §6），不可删
mount -o rw $ROOT /mnt || 应急 shell
exec switch_root /mnt /init            # busybox 会把 /proc /sys /dev 挂载点搬进新根
```

### 4.2 阶段 2：`infra/init`（测试 init，进入 rootfs 后的 PID 1）

- `mount_filesystems`：重挂 proc/sys/dev/devpts/shm——**必须幂等**
  （switch_root 后大多已是挂载态，报错要吞掉）；同时保证 rootfs 被直启
  （root 驱动 =y 的内核、无 initramfs）时也能自举；
- `load_modules`：从 rootfs 的 `/lib/modules/modules.conf` insmod 测试模块，
  行格式 `<module> [key=val ...]`（参数原样传给 insmod）；
- 无 cmdline `auto_test` → `setsid cttyhack /bin/sh` 交互 shell；
  有 → 依次执行 `/tests/*`，汇总 `Test Results: N/M` +
  `TEST_COMPLETE: ALL/SOME TESTS FAILED/PASSED`（CI 断言此标记）→ `poweroff -f`。

## 5. 模块双清单规则

| 清单 | 语义 | 去向 | 谁加载 | 来源 |
|---|---|---|---|---|
| `infra/modules-boot.conf` | 让 root= 可挂载的最小集（**冻结基础集，手工维护**） | initramfs `/lib/modules/` | `init-initramfs`，pivot 前 | 手写 conf 文件 |
| 组件 `stage = "boot"` 附加 | 某组件需要引导早期可用 | 同上（追加在基础集后） | 同上 | `virtuoso.toml` `[components.*] require` |
| 生成 `modules.conf` | 启用组件的一切模块需求 | rootfs.img `/lib/modules/` | `infra/init`，pivot 后 | 组件 require 并集（builder 生成） |

**判断标准：这个模块是不是"root 挂上之前就必须在内核里"？是 → 组件加
`stage = "boot"`，否 → 缺省 runtime。**require 条目 = conf 行
`"<module> [key=val ...]"`；并集按组件固定顺序去重保首个，条目顺序即
insmod 顺序（被依赖者在前）。缺 `.ko` 构建期报错。

组件机制总览见[组件机制](../components/overview.md)。

## 6. 已知内核怪癖与必要 workaround（不可删！）

针对 openEuler 内核 + 修改版 QEMU（含 UBIOS 日志）实测发现。删掉任何一个都会
回到 `cannot mount root=/dev/vda` 或 panic：

| # | 现象 | 根因 | 兜底（位置） |
|---|---|---|---|
| A | initramfs 里 `/dev` 是空的 | devtmpfs 挂上了但不自动填充 | `mknod console/null` + `mdev -s`（init-initramfs 开头） |
| B | `/sys/class/block/vda/dev` 内容为空 | sysfs dev 属性不输出 | 从 `/proc/partitions` 解析 major:minor 后 `mknod`（init-initramfs 中段） |
| C | 裸 ext4 root 直接 panic | `CONFIG_VIRTIO_BLK`/`CONFIG_EXT4_FS` = m | 两阶段架构本身（§1） |
| D | busybox 1.36.1 `tc` applet 编不过 | 内核头 ≥ 6.8 不兼容 | CI 里 `sed` 关掉 CONFIG_TC（workflow） |

## 7. 常见修改任务手册

**加一个测试模块**（最常见）
→ 在 `virtuoso.toml` 里给对应组件加/启用 `require`（注意依赖顺序，被依赖者
在前），`virtuoso build`。**不要**碰 `modules-boot.conf`——那会让
initrd.img 无谓变化。

**加测试用例** → 见[编写测试用例](../guide/writing-tests.md)；
改测试流程 → 改 `infra/init`。验证：`virtuoso build && virtuoso test --timeout 30`。

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
`busybox sh -n infra/init-initramfs`）。改完重跑 §8 的三条验证。

**换 rootfs 发行版**（如 Alpine/debootstrap）
→ rootfs.img 就是普通 ext4。改 builder 的 rootfs 组装段
（`crates/builder/src/`），或在 VM 外 loop mount 后替换内容。两阶段启动链
不需要任何改动。

**发新 busybox 版本** → Actions 手动触发 `busybox-release`（填版本号），
然后 `virtuoso.toml` 里 `[busybox] version = <ver>`。

**改 release rootfs 的 /init**
→ 原则上不要改（冻结不变量，§2.1）。确需修改：编辑 workflow 里 printf 段落，
重跑 workflow。**用 printf，不要用 heredoc**——YAML 块标量的缩进剥离会让
`EOF` 终止符对不上（dash 的 `<<-` 只认 tab，而 YAML 禁 tab），CI 会报
`syntax error: end of file unexpected`。

**改 shell 脚本通用规则**：GitHub Actions 的 `run:` 用 dash 执行——
`local a=1 b=$a` 同行互引会取到空值（必须分行）、不支持花括号展开
(`{a,b}`)、不支持 `${@:2}`（用 `shift`）。VM 内脚本（`init`、
`init-initramfs`）以 busybox ash 为准，同样按 POSIX 写，改完先做
`busybox sh -n` 语法校验。

## 8. 验证手册（改动后必跑）

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

## 9. 文件索引

| 文件 | 角色 | gitignored |
|---|---|---|
| `infra/init-initramfs` | 阶段 1 PID 1（pivot init） | — |
| `infra/init` | 阶段 2 PID 1（测试 init，构建时注入 rootfs） | — |
| `infra/modules-boot.conf` | boot 冻结基础集 → initramfs | — |
| `virtuoso.toml` | 唯一配置面：全局键 + 组件 require 并集 → rootfs modules.conf | — |
| `crates/builder/` | busybox 供给 + 两段式镜像组装 | — |
| `target/artifacts/initrd.img` / `rootfs.img` / `tools.img` | 构建产物 | ✅ |
| `target/build/busybox/`、`rootfs/`、`initramfs/`、`tools/` | 缓存与暂存目录 | ✅ |
| `.github/workflows/busybox-release.yml` | release 生成（含冻结 init） | — |
