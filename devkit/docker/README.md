# devkit/docker/ —— 容器化内核开发环境

**容器只做宿主做不到/不便做的事**：钉死内核构建工具链、在大小写敏感的
容器侧 ext4 上构建。源码权威存放在 **named volume**，宿主经平台视图直接
读写——AI、编辑器、git 全部原生工作，只有 `make` 进容器。测试资产构建
（busybox / 用例 workspace）与 QEMU 运行始终留在宿主原生（macOS 走 HVF、
Linux 走 KVM，容器里都没有）。

双平台同一套命令，唯一分支是 volume 的宿主可见路径：

| | macOS（OrbStack） | Linux（docker-ce） |
|---|---|---|
| volume 宿主路径 | `~/OrbStack/docker/volumes/<卷>`（OrbStack 视图，读写） | `/var/lib/docker/volumes/<卷>/_data`（volume 本体；rootless 落在 `$HOME` 下免 root） |
| `kernel.sh path` 输出 | 同上 | 同上（取引擎权威 Mountpoint） |
| QEMU 加速 | HVF | KVM |

Docker Desktop 的 volume 藏在 VM 虚拟盘里宿主不可见——macOS 上请用
OrbStack；用它又不想换引擎时走 `kernel.sh export` 回退（见下）。

| 文件 | 作用 |
|---|---|
| `Dockerfile.kernel` | 钉死工具链：ubuntu:24.04 + 内核构建依赖 + 三架构 gcc + clangd |
| `kernel.sh` | 薄壳：`clone / defconfig / menuconfig / build / cc / ccr / ccfix / path / export / shell`，每行都是可见的 `docker run` 拼装 |
| `devcontainer.json` | （可选）VS Code「Open Folder in Container」→ volume 为 workspace 的容器内编辑路径 |
| `.clangd` | flag 清洗配置，`clone` 时按目标架构写进源码根 |

## 为什么源码在 named volume 里

1. **大小写敏感**：Linux 内核构建要求大小写敏感文件系统；macOS 的 APFS
   默认不敏感。volume 是容器侧 ext4，天然正确。
2. **性能**：volume 的 I/O 走容器原生文件系统，构建速度接近原生 Linux
   （bind-mount 走 virtiofs 慢 2–5 倍，别用）。
3. **多内核切换**：每卷自含源码 + `.config` + 增量产物，切换 = 换卷名，
   切回旧内核免重编；卷的创建成本接近零（clone 是 `--depth 1`）。

## 标准流程（首跑）

```bash
# 0) 依赖：brew install qemu e2fsprogs dtc cmake zig + OrbStack（macOS）
#    Linux: 发行版 docker + qemu
# 1) 内核源码进 volume（+ 按架构配好 .clangd）
devkit/docker/kernel.sh clone https://gitee.com/openeuler/kernel.git --ref OLK-6.6-dev

# 2) 配置 + 构建（arm64 容器 = 原生前端，其余交叉）
devkit/docker/kernel.sh defconfig openeuler_defconfig
devkit/docker/kernel.sh build            # Image + modules + compile_commands.json

# 3) 内核树对宿主可见：`kernel.sh path` 打印的目录就是它
devkit/docker/kernel.sh path             # → 宿主可见路径
```

然后 `virtuoso.toml` 里 `kernel_path` 指向 **`kernel.sh path` 的输出**
（OrbStack 视图 / Linux volume 本体都是纯宿主路径，virtuoso 直接读
Image 与 `.ko`，无需导出），跑宿主主循环：

```bash
virtuoso doctor && virtuoso build && virtuoso test
```

**export 回退**：volume 没有宿主视图时（Docker Desktop on macOS、CI 等），
`kernel.sh export` 导出最小树（Makefile/.config/Image/\*\*/\*.ko →
`target/kernel/<arch>/`），`kernel_path` 指它。

## 多内核切换

```bash
KERNEL_VOLUME=ksrc-openEuler-6.6 devkit/docker/kernel.sh clone <url> --ref OLK-6.6-dev
KERNEL_VOLUME=ksrc-mainline      devkit/docker/kernel.sh clone <url> --ref master

KERNEL_VOLUME=ksrc-mainline devkit/docker/kernel.sh build   # 之后所有子命令同卷
```

切换 = `KERNEL_VOLUME` 换名（或给每个内核起一个 shell alias）。
`virtuoso.toml` 的 `kernel_path` 跟着指向对应卷的宿主路径。

## AI 闭环（单 AI 单工作区）

AI（及人的编辑器）的工作目录 = `kernel.sh path` 的输出——volume 的宿主
视图上一切原生：Read/Edit、git、grep、clangd（见下）。构建是 AI 眼里的
一条普通 shell 命令（`kernel.sh build`），验证在宿主：

```bash
cd "$(devkit/docker/kernel.sh path)"
# ……AI 改内核代码……
devkit/docker/kernel.sh build                  # 容器 make（volume 原生 I/O）
virtuoso doctor && virtuoso build && virtuoso test && virtuoso triage
```

判定以 `triage` 的 verdict 为准（`verdict: passed` 才算通过）；配合
`virtuoso skill install`（KERNEL_PATH 指向宿主可见路径）注入 kernel-dev /
kernel-virtuoso 两个 skill，AI 即具备驱动测试回路 / 分诊 / 写用例的知识。

## clangd / 浏览内核源码

`build` 产出的 `compile_commands.json` 已改写为**宿主路径形态**
（`/ksrc` → `kernel.sh path`），宿主 clangd 直接消费：VS Code 打开
`kernel.sh path` 的目录即得跳转/补全/悬停。改了 `.config` 或增量构建后
索引滞后时 `kernel.sh cc` 重生成。

CDB 的两种形态（构建在容器 `/ksrc` 下进行，宿主路径靠改写）：

| 子命令 | CDB 形态 | 消费方 |
|---|---|---|
| `build` / `cc` / `ccfix` | 宿主路径 | 宿主 clangd（主路径） |
| `ccr` | 容器 `/ksrc` 原始 | devcontainer 内 clangd |

- **VS Code（宿主，推荐）**：打开 `kernel.sh path` 的目录，装 clangd 扩展即可。
- **VS Code（devcontainer，可选）**：`Dev Containers: Open Folder in
  Container…` → 选本 `devkit/docker/` 目录（先 `kernel.sh ccr`）。
- **neovim / 其它 LSP**：宿主直接起 clangd（stdio）接自己的 LSP 通道；
  或 `kernel.sh shell` 进容器用容器内 clangd。

## 镜像发布

`kernel-builder.yml` 在 `devkit/docker/` 变更推 main 时构建并发布
`ghcr.io/yxalix/virtuoso-kernel`（linux/arm64 + linux/amd64）。
`kernel.sh` 拉不动镜像时自动回落本地构建。

## 换目标架构

```bash
KERNEL_ARCH=riscv64 devkit/docker/kernel.sh clone <url>
KERNEL_ARCH=riscv64 devkit/docker/kernel.sh build
```
