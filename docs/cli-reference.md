# CLI 参考

`virtuoso` 是唯一 CLI 入口（`cargo install --path xtask` 装入 PATH；工作区内
`cargo xtask` / `cargo v` 别名等价且改源码即重编）。`Makefile` 的每个 target
一一转发到对应子命令，语义与退出码不变（`virtuoso parity <target>` 可对照）。

## 命令一览

| 命令 | 层 | 说明 |
|---|---|---|
| `verify [--arch a]` | builder | 前置检查 + 类型化配置诊断 |
| `doctor [--arch a] [--json]` | builder | 同一检查引擎的 flutter-doctor 风格一屏体检（✓/✗/! 组件行）；报 ✗ 时用 verify 看全量 |
| `build` | builder | 重建 initrd.img / rootfs.img / tools.img |
| `fetch [--version v] [--arch a]` | builder | 拉取 preset 预编内核（mainline mini Image）到 target/kernel/preset；kernel_preset = "mainline" 的开箱供给 |
| `busybox` | builder | 确保当前架构静态 BusyBox（Release 下载优先，源码兜底） |
| `clean` | builder | 清理生成镜像与暂存目录 |
| `shell [--kvm] [--tcg]` | launcher | 交互式 VM（BusyBox shell）；`--kvm` 仅 Linux，`--tcg` 强制纯模拟（macOS 缺省 HVF 时用） |
| `debug` | launcher | 挂起启动 + GDB stub `:1234` |
| `test [--timeout N] [--arch a] [--replay-until-fail N] [--tcg]` | 全链路 | 构建 → 启动 → 判定 → 工件落盘；返场模式首个非 passed 即停；macOS 同构缺省 HVF |
| `matrix [--arch a]` | launcher | 多架构矩阵（缺省三架构，宿主内串行） |
| `probe --cmd/--cmd-file [--json] [--timeout N]` | launcher+judge | AI 交互通道：virtio-serial agent 命令批，结构化事件流 |
| `triage [--run id] [--json]` | runs | 最近（或指定）run 的分诊报告 |
| `runs [--json]` | runs | 历史运行列表 |
| `replay --log f [--json]` | judge | 任意串口日志的离线标记协议断言（不启动 QEMU） |
| `cluster [--json]` | tracker | 跨 run 失败指纹聚类 + flaky 清单 + 首现 run |
| `suggest [--diff f] [--json]` | tracker | git diff 子系统路径 → 推荐最小测试集 |
| `skill install \| uninstall` | xtask | AI skill 装入 / 移出内核树 |
| `docs [--serve] [--open]` | xtask | mdBook 文档构建到 target/book / 本地预览 |
| `parity <target> [--force] [--strict]` | xtask | make 与 virtuoso 行为对照（退出码三态判定） |

## 通用行为

- `--arch` 覆盖 `virtuoso.toml` 的同名键（等价环境变量 `ARCH`，
  见[配置参考](guide/configuration.md)）；
- `--json` 输出可直接进管道（jq 等）；
- 退出码语义：0=通过、124=超时（137 归一）、130=Ctrl-C、其余=失败——
  详见[冻结契约](architecture/contracts.md)；
- Ctrl-C 守护全局装载（guardian）：收割 QEMU 进程组、落盘已产出工件。

## AI 的标准验证循环

```
doctor → test → triage
```

判定以 triage 的 verdict 为准（`verdict: passed` 才算通过）。工作流详见
[AI 集成](guide/ai-integration.md)。
