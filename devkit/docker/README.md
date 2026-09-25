# devkit/docker/ —— 容器化内核开发环境

**容器只做宿主做不到/不便做的事**：钉死内核构建工具链、在大小写敏感的
容器侧 ext4 上构建。源码权威存放在 **named volume**，宿主经平台视图直接
读写——AI、编辑器、git 全部原生工作，只有 `make` 进容器。测试资产构建
（busybox / 用例 workspace）与 QEMU 运行始终留在宿主原生（macOS 走 HVF、
Linux 走 KVM，容器里都没有）。

命令面已收编进 CLI（原 `kernel.sh` 薄壳退役）：`virtuoso kernel` 子命令组
（逻辑在 `crates/forge`）。双平台同一套命令，唯一分支是 volume 的宿主可见
路径：

| | macOS（OrbStack） | Linux（docker-ce） |
|---|---|---|
| volume 宿主路径 | `~/OrbStack/docker/volumes/<卷>`（OrbStack 视图，读写） | `/var/lib/docker/volumes/<卷>/_data`（volume 本体；rootless 落在 `$HOME` 下免 root） |
| `virtuoso kernel path` 输出 | 同上 | 同上（取引擎权威 Mountpoint） |
| QEMU 加速 | HVF | KVM |

Docker Desktop 的 volume 藏在 VM 虚拟盘里宿主不可见——macOS 上请用
OrbStack（doctor 会守卫引擎端点）；用它又不想换引擎时走
`virtuoso kernel export` 回退（见下）。

| 文件 | 作用 |
|---|---|
| `Dockerfile.kernel` | 钉死工具链：ubuntu:24.04 + 内核构建依赖 + 三架构 gcc + clangd |
| `devcontainer.json` | （可选）VS Code「Open Folder in Container」→ volume 为 workspace 的容器内编辑路径 |

> `.clangd` 模板已内嵌 `crates/forge/src/kernel.clangd`，`clone` 时按目标
> 架构渲染写进源码根。

## 为什么源码在 named volume 里

1. **大小写敏感**：Linux 内核构建要求大小写敏感文件系统；macOS 的 APFS
   默认不敏感。volume 是容器侧 ext4，天然正确。
2. **性能**：volume 的 I/O 走容器原生文件系统，构建速度接近原生 Linux
   （bind-mount 走 virtiofs 慢 2–5 倍，别用）。
3. **多内核切换**：每卷自含源码 + `.config` + 增量产物，切换 = 换卷名，
   切回旧内核免重编；卷的创建成本接近零（clone 是 `--depth 1`）。

## 标准流程（首跑）

```bash
# 0) 依赖：brew install qemu e2fsprogs dtc zig + OrbStack（macOS）
#    Linux: 发行版 docker + qemu
# 1) 内核源码进 volume（+ 按架构渲染 .clangd + 写 current）
virtuoso kernel clone https://gitee.com/openeuler/kernel.git --ref OLK-6.6

# 2) 配置 + 构建（arm64 容器 = 原生前端，其余交叉）
virtuoso kernel defconfig openeuler_defconfig
virtuoso kernel build              # Image/bzImage + modules + compile_commands.json

# 3) 内核树对宿主可见：`virtuoso kernel path` 打印的目录就是它
virtuoso kernel path
```

然后 `virtuoso.toml` 里 `kernel_path` 指向 **`virtuoso kernel path` 的输出**
（OrbStack 视图 / Linux volume 本体都是纯宿主路径，virtuoso 直接读
Image 与 `.ko`，无需导出），跑宿主主循环：

```bash
virtuoso doctor && virtuoso build && virtuoso test
```

**export 回退**：volume 没有宿主视图时（Docker Desktop on macOS、CI 等），
`virtuoso kernel export` 导出最小树（Makefile/.config/Image/\*\*/\*.ko →
`target/kernel/<arch>/`），`kernel_path` 指它。

## 卷管理与多内核切换

```bash
virtuoso kernel clone <url> --ref OLK-6.6 --as ksrc-openEuler-6.6
virtuoso kernel clone <url> --ref master      --as ksrc-mainline

virtuoso kernel list        # 全部卷 + 内容状态（empty/cloned/configured）+ current 标记
virtuoso kernel use ksrc-mainline   # 切 current（写 .virtuoso/kernel-current.json）
```

`current` 指针是专门的会话状态文件 `.virtuoso/kernel-current.json`
（git 忽略）：`use`/`clone` 写入，之后所有 `virtuoso kernel` 子命令都作用
于它（`KERNEL_VOLUME` env 可临时压过，语义与其它键一致：env > 状态文件 > 缺省）。
`virtuoso.toml` 的 `kernel_path` 跟着指向对应卷的宿主路径。

## AI 闭环（单 AI 单工作区）

AI（及人的编辑器）的工作目录 = `virtuoso kernel path` 的输出——volume 的
宿主视图上一切原生：Read/Edit、git、grep、clangd（见下）。构建是 AI 眼里的
一条普通 shell 命令（`virtuoso kernel build`），验证在宿主：

```bash
cd "$(virtuoso kernel path)"
# ……AI 改内核代码……
virtuoso kernel build                           # 容器 make（volume 原生 I/O）
virtuoso doctor && virtuoso build && virtuoso test && virtuoso triage
```

判定以 `triage` 的 verdict 为准（`verdict: passed` 才算通过）；配合
`virtuoso skill install`（KERNEL_PATH 指向宿主可见路径）注入 kernel-dev /
kernel-virtuoso 两个 skill，AI 即具备驱动测试回路 / 分诊 / 写用例的知识。

## clangd / 浏览内核源码

`build` 产出的 `compile_commands.json` 已改写为**宿主路径形态**
（`/ksrc` → `virtuoso kernel path`），宿主 clangd 直接消费：VS Code 打开
`virtuoso kernel path` 的目录即得跳转/补全/悬停。改了 `.config` 或增量构建后
索引滞后时 `virtuoso kernel cc` 重生成。

CDB 的两种形态（构建在容器 `/ksrc` 下进行，宿主路径靠改写）：

| 子命令 | CDB 形态 | 消费方 |
|---|---|---|
| `kernel build` / `cc` / `ccfix` | 宿主路径 | 宿主 clangd（主路径） |
| `kernel ccr` | 容器 `/ksrc` 原始 | devcontainer 内 clangd |

- **VS Code（宿主，推荐）**：打开 `virtuoso kernel path` 的目录，装 clangd 扩展即可。
- **VS Code（devcontainer，可选）**：`Dev Containers: Open Folder in
  Container…` → 选本 `devkit/docker/` 目录（先 `virtuoso kernel ccr`）。
- **neovim / 其它 LSP**：宿主直接起 clangd（stdio）接自己的 LSP 通道；
  或 `virtuoso kernel shell` 进容器用容器内 clangd。

## 镜像发布

`kernel-builder.yml` 在 `devkit/docker/` 变更推 main 时构建并发布
`ghcr.io/yxalix/virtuoso-kernel`（linux/arm64 + linux/amd64）。
`virtuoso kernel` 拉不动镜像时自动回落本地构建 `Dockerfile.kernel`；
镜像可用 `KERNEL_TOOLCHAIN_IMAGE` 覆盖（`KERNEL_IMAGE` 与此无关——那是
virtuoso 启动内核镜像的配置键）。

## 换目标架构

架构缺省取 `virtuoso.toml` 的顶层 `arch`（测哪个编哪个），
`KERNEL_ARCH` env 或 `--arch` 临时覆盖：

```bash
virtuoso kernel clone <url> --arch riscv64
virtuoso kernel build --arch riscv64
```
