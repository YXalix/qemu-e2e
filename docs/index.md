# Virtuoso — 内核 E2E 虚拟化测试装置

Virtuoso 把 freshly-built 内核直接启动在 QEMU 里跑真实测试程序，用一条命令
回答 **"does my patch actually work?"**——供给内核 → 构建测试固件 → 启动跑
`/tests/` → 串口标记协议判定 → verdict 落盘。

> *内核是乐器，Virtuoso 是演奏家——每个 patch 都值得一场完整的独奏。*

设计理念、命名故事与架构总览见[设计与理念](concepts/design.md)。

## 最短路径

```bash
virtuoso doctor             # 一屏环境体检（✓/✗ 组件行；--verbose 全量）
virtuoso test --timeout 60  # 构建 → 启动 → 判定 → 工件落盘（收尾打印 verdict 行）
```

从零上手见[快速开始](quick-start.md)。

## 四阶段

| 阶段 | 命令 | 一句话 |
|---|---|---|
| 供给 | `virtuoso kernel …` | 容器化内核构建：named volume 源码 + 钉死工具链，产出 Image + `.ko` |
| 构建 | `virtuoso build` | 组装三个镜像：initrd.img / rootfs.img / tools.img |
| 启动 | `virtuoso shell` / `test` | QEMU 两段式引导（initramfs → rootfs），进程组全程受治 |
| 判定 | `virtuoso test` | 串口标记协议 v1 对账 → `verdict.json` 落盘（唯一判定事实） |

## 文档地图

| 部分 | 内容 |
|---|---|
| [入门](quick-start.md) | 从零到第一个 `verdict: passed`：装依赖 → 构建内核 → 体检 → 首跑 |
| [使用](usage/cli.md) | [CLI 参考](usage/cli.md)、[配置](usage/configuration.md)、[编写测试](usage/writing-tests.md)、[调试](usage/debugging.md)、[工件与分诊](usage/artifacts.md)、[AI 集成](usage/ai-integration.md)、[故障排查](usage/troubleshooting.md) |
| [概念与契约](concepts/design.md) | [设计与理念](concepts/design.md)、[供给与构建流水线](concepts/pipeline.md)、[两段式引导](concepts/boot.md)、[冻结契约](concepts/contracts.md)、[组件机制](concepts/components.md) |
| [参与](contributing.md) | 贡献约定与 CI 流水线 |

面向 AI 的仓库地图与不变量在根目录 [AGENTS.md](https://github.com/YXalix/virtuoso/blob/main/AGENTS.md)。
