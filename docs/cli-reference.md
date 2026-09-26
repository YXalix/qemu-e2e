# CLI 参考

`virtuoso` 是唯一 CLI 入口（`cargo install --path .` 装入 PATH）。

## 命令一览

| 命令 | 层 | 说明 |
|---|---|---|
| `doctor [--arch a] [--verbose] [--json]` | builder | 环境体检：一屏 ✓/✗/! 组件行；`--verbose` = 类型化配置诊断 + 完整检查清单 |
| `build [--busybox-only]` | builder | 重建 initrd.img / rootfs.img / tools.img；`--busybox-only` 仅备当前架构静态 BusyBox（Release 下载优先，源码兜底） |
| `kernel clone/defconfig/build/path/shell` | forge | 容器化内核供给：named volume 源码 + 钉死工具链镜像（详见 [devkit/docker/README.md](../devkit/docker/README.md)） |
| `kernel list` / `kernel use <volume>` | forge | 卷管理：列卷（状态 + current 标记）/ 切 current（状态文件 `.virtuoso/kernel-current.json`） |
| `clean` | builder | 清理生成镜像与暂存目录 |
| `shell [--kvm] [--tcg] [--gdb]` | launcher | 交互式 VM（BusyBox shell）；`--kvm` 仅 Linux，`--tcg` 强制纯模拟（macOS 缺省 HVF 时用），`--gdb` 挂起启动 + GDB stub `:1234`（恒 TCG） |
| `test [--timeout N] [--arch a] [--replay-until-fail N] [--tcg] [--only a,b]` | 全链路 | 构建 → 启动 → 判定 → 工件落盘；返场模式首个非 passed 即停；macOS 同构缺省 HVF；`--only` 只跑名单内 /tests 二进制（逗号分隔/可重复，零命中判失败） |
| `probe --cmd/--cmd-file [--json] [--timeout N]` | launcher+judge | AI 交互通道：virtio-serial agent 命令批，结构化事件流 |
| `skill install \| uninstall` | cli | AI skill 装入 / 移出内核树 |

## 通用行为

- `--arch` 覆盖 `virtuoso.toml` 的同名键（等价环境变量 `ARCH`，
  见[配置参考](guide/configuration.md)）；
- `--json` 输出可直接进管道（jq 等）；
- 退出码语义：0=通过、124=超时（137 归一）、130=Ctrl-C、其余=失败——
  详见[冻结契约](architecture/contracts.md)；
- Ctrl-C 守护全局装载（launcher::guardian）：收割 QEMU 进程组、落盘已产出工件。

## AI 的标准验证循环

```
doctor → test
```

判定以 test 收尾的 verdict 行为准（`verdict: passed` 才算通过；机读唯一面 =
run 目录下的 `verdict.json`）。工作流详见 [AI 集成](guide/ai-integration.md)。
