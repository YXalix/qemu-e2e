# docker/ —— 容器化内核开发环境

**容器只做宿主做不到的事**（编 Linux 内核 + clangd 索引环境）；测试资产
构建（busybox / 用例 workspace）与 QEMU 运行留在宿主原生——macOS 上
QEMU 走 HVF 近原生速度，容器里没有（嵌套虚拟化不可用），塞进去等于自废。

| 文件 | 作用 |
|---|---|
| `Dockerfile.kernel` | 钉死工具链：ubuntu:24.04 + 内核构建依赖 + 三架构 gcc + clangd |
| `kernel.sh` | 薄壳：`clone / defconfig / menuconfig / build / cc / export / shell` |
| `devcontainer.json` | VS Code「Open Folder in Container」→ 内核源码 volume 为 workspace |
| `.clangd` | flag 清洗配置，`clone` 时按目标架构写进源码根 |

## 为什么源码在 named volume 里

1. **大小写敏感**：Linux 内核构建要求大小写敏感文件系统；macOS 的 APFS
   默认不敏感，bind-mount 进容器也救不回来。volume 是容器侧 ext4，天然正确。
2. **性能**：volume 的 I/O 走容器原生文件系统，比 bind-mount 经历一次
   virtiofs 转发快。

编辑体验不受影响：VS Code 的 server 跑在容器内、直接读写 volume；
neovim 党 `kernel.sh shell` 进容器，用容器内 clangd。

## 标准流程（macOS 首跑）

```bash
# 0) 依赖：brew install qemu e2fsprogs dtc cmake zig + Docker Desktop/OrbStack
# 1) 内核源码进 volume（+ 按架构配好 .clangd）
docker/kernel.sh clone https://gitee.com/openeuler/kernel.git --ref OLK-6.6-dev

# 2) 配置 + 构建（arm64 在 arm64 容器内 = 原生速度）
docker/kernel.sh defconfig openeuler_defconfig
docker/kernel.sh build            # Image + modules + compile_commands.json

# 3) 导出最小树（Makefile/.config/Image/**/*.ko → target/kernel/arm64/）
docker/kernel.sh export

# 4) virtuoso 主循环（宿主原生，HVF）
virtuoso.toml 里 kernel_path = "target/kernel/arm64"
virtuoso doctor && virtuoso build && virtuoso test
```

## 浏览内核源码（clangd in Docker）

构建过一次后 `compile_commands.json` 已在源码根（内核自带
`scripts/compile_commands.py` 生成，无需 bear）：

- **VS Code**：命令面板 → `Dev Containers: Open Folder in Container…`
  → 选本 `docker/` 目录。workspace 即 `/ksrc`（volume），clangd 扩展自动
  装，跳转/补全/悬停全量可用。
- **neovim / 其它 LSP 客户端**：`docker/kernel.sh shell` 进容器，起
  clangd（stdio 模式）经自己的 remote-LSP 通道接出。

改了 `.config` 或增量构建后索引滞后时：`docker/kernel.sh cc` 重生成。

## 镜像发布

`kernel-builder.yml` 在 `docker/` 变更推 main 时构建并发布
`ghcr.io/yxalix/virtuoso-kernel`（linux/arm64 + linux/amd64）。
`kernel.sh` 拉不动镜像时自动回落本地构建。

## 换目标架构

```bash
KERNEL_ARCH=riscv64 docker/kernel.sh clone <url>
KERNEL_ARCH=riscv64 docker/kernel.sh build && KERNEL_ARCH=riscv64 docker/kernel.sh export
```
