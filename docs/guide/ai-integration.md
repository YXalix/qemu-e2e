# AI 集成

Virtuoso 对 AI 代理是一等公民：结构化事件流做事实源、skill 做知识注入、
probe 做运行时交互。

## 标准验证循环

```
doctor → test → triage
```

**判定以 triage 的 verdict 为准**：`verdict: passed` 才算通过。退出码只是
接口契约（0=通过、124=超时、其余=失败）——`-no-reboot` 下内核 panic 会让
QEMU 以 exit 0 退出，只看退出码会假通过。

## 数据接口

| 能力 | 输入 | 输出 | 对接点 |
|---|---|---|---|
| **测试脚手架生成** | 自然语言描述 / git diff | 用例 crate（Rust 入口 + C 体） | builder 编译即用 |
| **串口日志分诊** | `events.jsonl` / `verdict.json` | 根因假设 + 建议复现命令 | `virtuoso triage` |
| **失败指纹聚类** | 跨 run 的 verdict/事件流 | flaky 清单 + 失败首现 run | `virtuoso cluster`（judge::tracker） |
| **VM 内交互探测** | shell 命令批 | 结构化事件流（`agent-events.jsonl`） | `virtuoso probe`（virtio-serial + virtuoso-agent） |

## skill

```bash
virtuoso skill install      # kernel-dev + kernel-virtuoso skill 装入内核树
virtuoso skill uninstall
```

- `kernel-dev`：教 AI 驱动测试回路 / 写用例 / 解析串口 / 分诊失败；
- `kernel-virtuoso`：AI 数据接口集成（probe 通道）。

装入后 AI 在内核树内自动发现（`.claude/skills/` 机制）。

## probe 通道

`virtuoso probe` 恒开 [agent 组件](../components/agent.md)通道（不依赖组件
开关），经 guest 侧 virtuoso-agent 下发命令批，结构化事件流回吐：

```bash
virtuoso probe --cmd 'uname -a' --cmd 'cat /proc/iomem' --json
```

## 安全边界（架构约束）

- AI **只读分析**测试产物与内核日志；对内核源码的任何修改必须人工确认后由
  开发者执行；
- `events.jsonl` 是 AI 的唯一结构化事实源，串口原文仅作补充上下文，保证分诊
  可回溯；
- skill 与 harness 的接口是[冻结契约](../architecture/contracts.md)：skill 只
  依赖协议与工件 schema，不依赖实现。
