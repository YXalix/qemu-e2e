# 快速开始

从零到第一个 `verdict: passed`：装依赖 → 构建被测内核 → 安装 virtuoso →
环境体检 → 首跑 → 分诊。

## 1. 前置条件

- 宿主工具：`rust`（stable）、`gcc` / `cmake`、`wget` / `cpio` / `gzip`、`qemu-system-*`
- 内核源码树（本装置设计为放进内核树内运行，如 `kernel/virtuoso/`）
- openEuler / Fedora：`sudo dnf install -y gcc make cmake wget cpio gzip qemu-system-aarch64 qemu-img`
- Debian / Ubuntu：`sudo apt install -y gcc make cmake wget cpio gzip qemu-system-arm qemu-utils`

## 2. 构建被测内核

```bash
git clone https://gitcode.com/openeuler/kernel.git && cd kernel
cp arch/arm64/configs/openeuler_defconfig .config   # 或自己的 .config
make -j"$(nproc)"
make modules -j"$(nproc)"        # 只要有任何模块是 =m
```

装置默认放在内核树内（自动探测）；放别处时在 `virtuoso.toml` 设 `kernel_path`。

## 3. 安装与体检

```bash
cargo install --path xtask    # 规范二进制 virtuoso 装入 PATH（一次）；工作区内 cargo xtask / cargo v 别名等价
virtuoso doctor               # 一屏体检：✓/✗/! 组件行，最快确认接线
```

`doctor` 报 ✗ 时用 `virtuoso verify` 看全量清单与类型化配置诊断（两者共用
同一检查引擎，只是呈现繁简之别，同参 `--arch` / `--backend`）。

## 4. 首跑与判定

```bash
virtuoso test --timeout 60    # 构建 → 启动 → 判定 → 工件落盘
virtuoso triage               # 分诊最近一次运行
```

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
