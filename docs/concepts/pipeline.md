# 供给与构建流水线

`virtuoso build` 把「内核树 + 测试用例 + 组件配置」组装成三个镜像。本文讲
供给链与流水线的概念、产物形态与不变量；镜像在 VM 内怎么被消费见
[两段式引导](boot.md)。

## 三个镜像

| 镜像 | 内容 | 生命周期 |
|---|---|---|
| `initrd.img` | busybox + **boot 模块** + pivot init（cpio.gz） | switch_root 后内存释放 |
| `rootfs.img` | busybox + 测试 init + `/tests/*` + 测试模块（ext4） | 持久，可 loop mount 随意改 |
| `tools.img` | `infra/tools/` 常驻工具（musl 静态，ext4 卷标 `tools`） | 持久；工具供给缺失时**不产出**——显式降级非掩盖 |

构建输入：`kernel_path`（内核树，找 `.ko`）、`arch`、`infra/` 资产、
`virtuoso.toml` 的组件 require 并集。产物落 `target/artifacts/`，BusyBox
缓存与暂存目录在 `target/build/`。

## BusyBox 供给链

静态 BusyBox 统一由 `busybox-release` workflow 预编译发布，本地不做源码编译。

### GitHub Release（通用产物，刻意保持稳定）

tag `busybox-v<version>`，每架构两个资产：

| 资产 | 内容 |
|---|---|
| `busybox-<ver>-linux-<arch>` | 裸静态 busybox 二进制（defconfig + `CONFIG_STATIC=y`） |
| `rootfs-<ver>-linux-<arch>.cpio.gz` | 通用 busybox rootfs 目录树（cpio.gz 形式） |

稳定性不变量：

- release rootfs 的 `/init` 是**冻结在 workflow 里的 10 行脚本**（printf
  拼接），不引用任何仓库脚本——改仓库任何 `.sh`/`init*` 都**不需要**重发
  release；资产内容只随 busybox 版本变化。
- release **不含**任何测试内容（无 `/tests`、无测试 init、无模块清单）。
  测试是本仓库的场景注入，由 `virtuoso build` 完成。
- rootfs 目录树：`bin/`（busybox + 全量 applet 符号链接）+ `sbin usr/{bin,sbin}`
  + 空骨架目录 + `/etc/passwd`、`/etc/group`（root）。
- applet 符号链接由名单驱动（`infra/busybox/applets-<version>.txt`，名单即
  x86_64 `busybox --list` 输出的冻结版），**不执行 guest 二进制**——交叉组装
  （含 macOS 宿主）无需宿主同构；自定义版本/配置走 `$BUSYBOX_APPLETS_FILE`。

重新发布：Actions 页手动 `workflow_dispatch`（默认 1.36.1），或推 `busybox-v*`
tag。发布是幂等的：同名 asset 先删后传，release 已存在则复用。

### 本地拉取（四级，无兜底）

`virtuoso build --busybox-only` 按序尝试，命中即缓存到
`target/build/busybox/bin/`（版本进缓存键，换版本即重新拉取）：

```text
0. 本地缓存  →  1. $BUSYBOX_DL_URL  →  2. gh release download（带认证，私有库可用）  →  3. 直链下载（ELF 魔数校验）
```

全部未命中即报错——**无源码编译兜底**（发布仓库推导：`$BUSYBOX_RELEASE_REPO` >
扫描 git remotes 找 github.com 的那个）。离线环境提前把对应架构的二进制放进
缓存目录即可。

### ext4 尺寸教训

绝不要拍固定尺寸——ext4 的元数据 + journal 会把创建时声明的大小全部真实占满
（曾有过 64MB 空洞的教训）。镜像尺寸 = 内容实测 + 余量，由构建自动计算。

## 组装流水线（概念）

```text
1. BusyBox 供给    → per-arch 静态 busybox（见上文四级拉取）
2. busybox 树      → 二进制 + 名单驱动 applet 符号链接 + 骨架目录 + passwd/group
3. initramfs 树    → busybox 树 + pivot init + boot 模块集
                     → cpio newc + gzip（mtime 恒 0 + 路径排序 → 产物确定性可复现）
4. rootfs 树       → busybox 树 + 测试 init + 测试模块清单 + /tests/*（musl 静态，含测试组子目录）
                     + init-hooks.sh + rootfs.d 增量并入（只增不覆盖）
                     → ext4（自动尺寸）
5. tools.img 树    → tools workspace 的 musl 静态产物 → ext4（卷标 tools）
```

- **测试用例（C over Rust）**：`infra/testcases/` 独立 workspace，coda（std
  Rust）为框架与统一入口，C 测试体经用例 crate 的 build.rs 编入同一 musl
  静态二进制；交叉编译统一走 `zig cc`（宿主差异被抹平）。用例编写见
  [编写测试用例](../usage/writing-tests.md)。
- **`init-hooks.sh` 注入点**：builder 生成 rootfs 内 `/init-hooks.sh`，init
  侧守卫 source（位于 devtmpfs 挂载后、insmod/agent 拉起前）——tools 盘挂载
  + PATH 注入即走此通道；VM 内定制写 hook 片段即可（例子见
  [两段式引导的修改手册](boot.md#5-修改任务手册)）。
- **检查引擎**：构建前置条件（工具链 / 内核镜像 / QEMU / 模块 / BusyBox
  缓存 / 组件状态）由类型化检查引擎承载；`doctor` 与构建共用同一引擎，
  新增检查只动引擎、两侧呈现自动跟随。

## 文件索引

| 文件 | 角色 |
|---|---|
| `infra/init` / `infra/init-initramfs` | 两阶段 PID 1（见[两段式引导](boot.md)） |
| `infra/modules-boot.conf` | boot 冻结基础模块集 → initramfs |
| `infra/testcases/`、`infra/tools/` | 用例与工具 workspace |
| `rootfs.d/` | 用户 drop-in（增量并入 rootfs，只增不覆盖；git 忽略，缺省可无） |
| `target/artifacts/*.img` | 构建产物（git 忽略） |
| `.github/workflows/busybox-release.yml` | release 生成（含冻结 init） |
