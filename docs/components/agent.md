# agent — AI probe 通道

宿主与 guest 之间的结构化交互通道：virtio-serial 串口 + guest 侧
`virtuoso-agent`（tools.img 内），用 JSON 行协议双向通信。这是
`virtuoso probe`（[AI 集成](../guide/ai-integration.md)）的底座。

## 配置

```toml
[components.agent]        # 段缺省 = 关闭（argv 保持冻结基线）
enabled = true
require = ["virtio_console"]
```

`virtuoso probe` **恒开** agent 通道（强制并入 `virtio_console`），不依赖
本组件开关——临时探测不用改配置。

## 工作方式

1. 启用后 launcher 追加 virtio-serial 设备参数（属调用方增量，追加在
   argv 冻结基线之后）；
2. guest 侧 `infra/init` 经 `/init-hooks.sh` 拉起 `/tools/virtuoso-agent`；
3. agent 在 virtio-serial 上收 JSON 行命令、回 JSON 行事件（结构化事件流，
   非模拟终端敲键盘）；
4. 宿主侧事件流落盘 run 目录的 `agent-events.jsonl`。

## 使用

```bash
virtuoso probe --cmd 'uname -a' --cmd 'dmesg | tail'
virtuoso probe --cmd-file cmds.txt --json    # 机器可读事件流，供 AI 管道消费
```

`--timeout` 覆盖墙钟总超时（含 TCG 引导与握手）；退出码语义 0/1/124。
probe 运行也写 run 目录（`serial.log` + `qemu-stderr.log` +
`agent-events.jsonl`，无 verdict）。

## 限制

- Firecracker 后端不支持 virtio-serial：组件启用时 WARN 忽略，probe 不可用。
- 通道依赖 tools.img 供给（agent 二进制在工具盘里）。
