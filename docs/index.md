# Virtuoso — 内核 E2E 虚拟化测试装置

Virtuoso 是内核 E2E 虚拟化测试装置：把 freshly-built 内核直接启动在
QEMU（或 Firecracker microVM）里跑真实测试程序，用一条命令回答
**"does my patch actually work?"**。

> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

## 最短路径

```bash
cargo xtask verify             # 前置检查 + 类型化配置诊断
cargo xtask test --timeout 60  # 构建 → 启动 → 判定 → 工件落盘
cargo xtask triage             # 以 verdict 为准的判定报告
```

## 文档地图

| 文档 | 内容 |
|---|---|
| [使用指南](user-guide.md) | 日常操作手册：上手、配置、写用例、模块、调试、组件 |
| [Initramfs 与 Rootfs 构建指南](initramfs-rootfs-guide.md) | 两段式引导逐行解读、资产供给链、"改哪个文件"手册、内核怪癖表 |
| [Troubleshooting](troubleshooting.md) | Symptom → Solution 速查 |
| [架构与设计](virtuoso-design.md) | 总体架构、核心模块设计、冻结契约、标记协议 v1（冻结文本） |

面向 AI 的仓库地图与不变量在根目录 [AGENTS.md](https://github.com/YXalix/virtuoso/blob/main/AGENTS.md)。

## 一句话架构

Rust workspace 是唯一行为权威：`common`（基础层）→ `builder`（构建）→
`launcher`（启动 DSL + QEMU/Firecracker 双后端）→ `judge`（判定）→
`guardian`（进程治理）→ `tracker`（跨 run 聚类），`xtask` 是 CLI 入口，
`Makefile` 是转发壳。每次运行落盘 `target/runs/<id>-<arch>/`，
`verdict.json` 是判定的唯一事实（exit code 不是——`-no-reboot` 下内核
panic 会让 QEMU 以 exit 0 退出）。
