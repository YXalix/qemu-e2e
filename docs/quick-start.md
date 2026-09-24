# 快速开始

从零到第一个 `verdict: passed`：装依赖 → 构建被测内核 → 安装 virtuoso →
环境体检 → 首跑 → 分诊。

## 1. 前置条件

- 宿主工具：`rust`（stable）、`zig`（C 测试用例编译器）、`make`、`qemu-system-*`
- 内核源码树（本装置设计为放进内核树内运行，如 `kernel/virtuoso/`）
- openEuler / Fedora：`sudo dnf install -y gcc make wget cpio gzip qemu-system-aarch64 qemu-img`，zig 从 [ziglang.org/download](https://ziglang.org/download/) 获取（发行版源一般不收录）
- Debian / Ubuntu：`sudo apt install -y gcc make zig wget cpio gzip qemu-system-arm qemu-utils`

> initrd 打包已 Rust 原生化（builder::cpio），`cpio` / `gzip` / `wget` 不再
> 是硬依赖（下载层有 curl 回退）——装了更省事，没装 doctor 也不报 ✗。

### macOS（Apple Silicon，M1–M5）前置

```bash
brew install qemu e2fsprogs dtc zig
```

- `qemu`：`qemu-system-aarch64` 带 HVF 加速（Apple Silicon 上近原生）；
- `e2fsprogs`：`mke2fs`（keg-only，virtuoso 自动探测 keg 路径，无需加 PATH）；
- `dtc`：pmem 组件的 dumpdtb/fdtput 工具（不用 pmem 可不装）；
- `zig`：C 测试用例交叉编译（统一编译器路径：`zig cc -target <musl triple>`
  三宿主一致，产 Linux 静态 ELF；testcases 已全走 cargo，无需 cmake）；
- Rust 侧另需 `rustup target add aarch64-unknown-linux-musl`。
- vfio 直通不支持 macOS（架构性依赖 Linux IOMMU），doctor 会直接拒绝。

## 2. 构建被测内核

```bash
git clone https://gitcode.com/openeuler/kernel.git && cd kernel
cp arch/arm64/configs/openeuler_defconfig .config   # 或自己的 .config
make -j"$(nproc)"
make modules -j"$(nproc)"        # 只要有任何模块是 =m
```

装置默认放在内核树内（自动探测）；放别处时在 `virtuoso.toml` 设 `kernel_path`。

### macOS 上的内核供给（docker/ 内核开发环境）

macOS 本机编不了 Linux 内核，用容器钉死工具链（Firecracker build-docker
式；源码放 named volume，规避 APFS 大小写不敏感坑）：

```bash
docker/kernel.sh clone https://gitcode.com/openeuler/kernel.git --ref OLK-6.6-dev
docker/kernel.sh defconfig openeuler_defconfig
docker/kernel.sh build          # Image + modules + clangd 索引数据
docker/kernel.sh export         # 最小树 → target/kernel/arm64/
```

然后在 `virtuoso.toml` 设 `kernel_path = "target/kernel/arm64"`。VS Code
经 devcontainer 打开内核源码 volume 即得 clangd 全量跳转/补全（构建一次
后自动有 compile_commands.json），详见 [docker/README.md](../docker/README.md)。

内核树也可在任何 Linux 机器上构建后 rsync 过来——virtuoso 只要求
`KERNEL_PATH` 指向「Makefile + arch/<a>/boot/<Image> + \*.ko」的树。

### 免内核树：preset 预编内核（最快路径）

只想先跑起来（或 CI 里验证装置本身）时，跳过整个第 2 节：

```toml
# virtuoso.toml
kernel_preset = "mainline"
```

```bash
virtuoso fetch                # 拉官方预编 mainline mini 内核（全 =y 零模块）
virtuoso test --timeout 300   # 直接开跑
```

注意定位：preset 内核验证的是**装置自身**（引导链路 + 判定）；被测内核
行为（openEuler 特有语义）仍以源码树路径为准。详见
[preset 内核](guide/kernel-preset.md)。

## 3. 安装与体检

```bash
cargo install --path xtask    # 规范二进制 virtuoso 装入 PATH（一次）；工作区内 cargo xtask / cargo v 别名等价
virtuoso doctor               # 一屏体检：✓/✗/! 组件行，最快确认接线
```

`doctor` 报 ✗ 时用 `virtuoso verify` 看全量清单与类型化配置诊断（两者共用
同一检查引擎，只是呈现繁简之别，同参 `--arch`）。

## 4. 首跑与判定

```bash
virtuoso test --timeout 60    # 构建 → 启动 → 判定 → 工件落盘
virtuoso triage               # 分诊最近一次运行
```

加速器语义按宿主平台：**Linux 恒 TCG**（`shell --kvm` 交互式开 KVM）；
**macOS 上宿主与目标同构（arm64）时缺省 HVF**，`--tcg` 强制纯模拟，
交叉 guest（x86_64 / riscv64）自动回落 TCG（慢，超时预算酌情放大）。

**判定以 triage 的 verdict 为准**：`verdict: passed` 才算通过。退出码只是
接口契约（0=通过、124=超时、其余=失败）——`-no-reboot` 下内核 panic 会让
QEMU 以 exit 0 退出，只看退出码会假通过；`exit 0` 但
`verdict: incomplete` = 标记协议没走完，同样按失败处理。

Verdict 全集（`judge::Verdict`）：`passed` / `failed` / `timeout` / `panic` /
`incomplete` / `interrupted` / `build_failed` / `unknown`。

## 5. 下一步

- 日常操作与全部命令：[CLI 参考](cli-reference.md)
- 改配置 / 换架构 / 开组件：[配置参考](guide/configuration.md)
- 写自己的测试用例：[编写测试用例](guide/writing-tests.md)
- 出了问题：[Troubleshooting](troubleshooting.md)
