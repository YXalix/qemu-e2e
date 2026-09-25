# Virtuoso — 内核 E2E 虚拟化测试装置

Virtuoso 是内核 E2E 虚拟化测试装置：把 freshly-built 内核直接启动在
QEMU 里跑真实测试程序，用一条命令回答
**"does my patch actually work?"**。

> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

## 最短路径

```bash
virtuoso doctor             # 一屏环境体检（✓/✗ 组件行；--verbose 全量）
virtuoso test --timeout 60  # 构建 → 启动 → 判定 → 工件落盘
virtuoso triage             # 以 verdict 为准的判定报告
```

从零上手见[快速开始](quick-start.md)。

## 文档地图

| 章节 | 内容 |
|---|---|
| [快速开始](quick-start.md) | 从零到第一个 `verdict: passed`：装依赖 → 构建内核 → 体检 → 首跑 |
| [架构](architecture/overview.md) | 设计原则、总体架构、workspace 结构；[核心 crate 设计](architecture/crates.md)；[冻结契约](architecture/contracts.md) |
| [组件](components/overview.md) | VM 能力组件机制与逐组件页：tools_disk / agent / vfio / numa / pmem |
| [使用指南](guide/configuration.md) | [配置](guide/configuration.md)、[编写测试](guide/writing-tests.md)、[调试](guide/debugging.md)、[运行工件与跨 run 分析](guide/artifacts.md)、[AI 集成](guide/ai-integration.md) |
| [内部机制](internals/boot-pipeline.md) | 两段式引导逐行解读、资产供给链、"改哪个文件"手册、内核怪癖表 |
| [参考](cli-reference.md) | [CLI 参考](cli-reference.md)、[Troubleshooting](troubleshooting.md)、[贡献约定](contributing.md) |

面向 AI 的仓库地图与不变量在根目录 [AGENTS.md](https://github.com/YXalix/virtuoso/blob/main/AGENTS.md)。

## 一句话架构

Rust workspace 是唯一行为权威：`common`（基础层）→ `builder`（构建）→
`launcher`（启动 DSL）→ `judge`（判定）→
`guardian`（进程治理）→ `tracker`（跨 run 聚类），入口 CLI 是 `virtuoso`（cli crate）。
每次运行落盘 `target/runs/<id>-<arch>/`，
`verdict.json` 是判定的唯一事实（exit code 不是——`-no-reboot` 下内核
panic 会让 QEMU 以 exit 0 退出）。
