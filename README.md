# Virtuoso — 内核 E2E 虚拟化测试装置

内核 E2E 虚拟化测试装置：把 freshly-built 内核直接启动在 QEMU 里跑真实测试
程序，一条命令回答 *"does my patch actually work?"*。

构建（builder）、启动（launcher）、判定（judge）、进程治理（guardian）、
跨 run 聚类（tracker）全部在类型化的 Rust workspace 中，`virtuoso` 是唯一
CLI 入口。

**📖 在线文档：<https://yxalix.github.io/virtuoso/>**

## 快速开始

从零上手见[快速开始](https://yxalix.github.io/virtuoso/quick-start.html)。
装置设计为放进内核源码树内运行（如 `kernel/virtuoso/`），放别处时在
`virtuoso.toml` 设 `kernel_path`。

```bash
cargo install --path .      # 规范二进制 virtuoso 装入 PATH
virtuoso doctor             # 一屏环境体检（✓/✗ 组件行；--verbose 全量）
virtuoso test --timeout 60  # 构建 → 启动 → 判定 → 工件落盘
virtuoso triage             # 分诊报告（判定以 verdict 为准）
```

## 常用命令

| 命令 | 作用 |
|---|---|
| `virtuoso doctor` | 一屏环境体检（`--verbose` 全量诊断，`--json` 机器可读） |
| `virtuoso build` | 重建 initrd.img / rootfs.img / tools.img |
| `virtuoso shell [--kvm] [--gdb]` | 交互式 VM（BusyBox shell）；`--gdb` GDB stub `:1234` 挂起启动 |
| `virtuoso matrix [--arch a]` | 多架构矩阵（x86_64 / arm64 / riscv64） |
| `virtuoso probe --cmd '…'` | AI 交互通道（virtio-serial agent 命令批） |
| `virtuoso cluster` / `suggest` | 跨 run 失败聚类 / 补丁→最小测试集 |
| `virtuoso docs [--serve]` | 文档构建 / 本地预览（mdBook） |

## 文档

`docs/` 是文档唯一事实来源，经 mdBook 发布到 GitHub Pages；本地
`virtuoso docs` 构建到 `target/book`，push main 自动更新站点。

- [快速开始](docs/quick-start.md) — 从零到第一个 `verdict: passed`
- [架构](docs/architecture/overview.md) — 总体架构、核心 crate 设计、[冻结契约](docs/architecture/contracts.md)
- [组件](docs/components/overview.md) — VM 能力组件：tools_disk / agent / vfio / numa / pmem
- [使用指南](docs/guide/configuration.md) — 配置、[编写测试](docs/guide/writing-tests.md)、[调试](docs/guide/debugging.md)、[运行工件与跨 run 分析](docs/guide/artifacts.md)
- [两段式引导与构建流水线](docs/internals/boot-pipeline.md) — 逐行解读、"改哪个文件"手册、内核怪癖表
- [Troubleshooting](docs/troubleshooting.md) — Symptom → Solution 速查
- [AGENTS.md](AGENTS.md) — AI 面向的仓库地图与不变量

## License

[MIT](LICENSE)。
