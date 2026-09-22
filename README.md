# Virtuoso — 内核 E2E 虚拟化测试装置

内核 E2E 虚拟化测试装置：把 freshly-built 内核直接启动在 QEMU / Firecracker
microVM 里跑真实测试程序，一条命令回答 *"does my patch actually work?"*。

构建（builder）、启动（launcher）、判定（judge）、进程治理（guardian）、
跨 run 聚类（tracker）全部在类型化的 Rust workspace 中，`cargo xtask` 是唯一
CLI 入口，`Makefile` 只是转发壳。

**📖 在线文档：<https://yxalix.github.io/virtuoso/>**

## 快速开始

前置条件与内核构建步骤见[使用指南](https://yxalix.github.io/virtuoso/user-guide.html)。
装置设计为放进内核源码树内运行（如 `kernel/virtuoso/`），放别处时在
`virtuoso.toml` 设 `kernel_path`。

```bash
cargo xtask verify             # 前置检查 + 类型化配置诊断
cargo xtask test --timeout 60  # 构建 → 启动 → 判定 → 工件落盘
cargo xtask triage             # 分诊报告（判定以 verdict 为准）
```

## 常用命令

| 命令 | 作用 |
|---|---|
| `cargo xtask build` | 重建 initrd.img / rootfs.img / tools.img |
| `cargo xtask shell [--kvm]` | 交互式 VM（BusyBox shell） |
| `cargo xtask debug` | GDB stub `:1234` 挂起启动 |
| `cargo xtask matrix [--arch a]` | 多架构矩阵（x86_64 / arm64 / riscv64） |
| `cargo xtask probe --cmd '…'` | AI 交互通道（virtio-serial agent 命令批） |
| `cargo xtask cluster` / `suggest` | 跨 run 失败聚类 / 补丁→最小测试集 |
| `cargo xtask docs [--serve]` | 文档构建 / 本地预览（mdBook） |

## 文档

`docs/` 是文档唯一事实来源，经 mdBook 发布到 GitHub Pages；本地
`cargo xtask docs` 构建到 `target/book`，push main 自动更新站点。

- [使用指南](docs/user-guide.md) — 上手、配置、写用例、模块、调试、组件
- [Initramfs 与 Rootfs 构建指南](docs/initramfs-rootfs-guide.md) — 两段式引导逐行解读、"改哪个文件"手册
- [Troubleshooting](docs/troubleshooting.md) — Symptom → Solution 速查
- [架构与设计](docs/virtuoso-design.md) — 总体架构、核心模块设计、冻结契约、标记协议 v1（冻结）
- [AGENTS.md](AGENTS.md) — AI 面向的仓库地图与不变量

## License

[MIT](LICENSE)。
