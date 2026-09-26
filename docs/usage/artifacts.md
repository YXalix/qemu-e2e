# 运行工件与分诊

每次 `virtuoso test` 写入 `target/runs/<unix_ms>-<arch>/`（保留最近
20 次）；`probe` 也写 run 目录（内容为 `serial.log` + `qemu-stderr.log` +
`agent-events.jsonl`，无 verdict）。这些工件是 AI 与人共用的分诊数据源。

## run 目录内容

| 文件 | 内容 |
|---|---|
| `serial.log` | QEMU stdout：内核串口 + 测试标记（含 panic 栈），逐行同步落盘 |
| `qemu-stderr.log` | QEMU 自身告警 |
| `build.log` | 本次构建输出（成功也保留） |
| `events.jsonl` | judge 逐行解析的结构化事件（test_start / test_end / assert / summary / marker / panic / oops / run_end） |
| `verdict.json` | 汇总判定 + 运行指纹（内核 mtime/大小、QEMU 版本、拓扑、超时） |

**终端呈现同样按流分离**（test / probe）：运行中终端只回显 guest
串口（即 `serial.log` 的内容）；QEMU 自身输出不刷屏，只落 `qemu-stderr.log`，
失败或超时收尾时回显其末尾至多 40 行（QEMU 早夭、参数被拒的现场就在这几行
里），成功保持安静。`virtuoso shell`（含 `--gdb` 调试挂起）是交互式会话，
stdio 天然合流，QEMU 输出仍实时可见。

## Verdict 语义

**判定以 verdict 为准，退出码只是接口契约**：`-no-reboot` 下内核 panic 会让
QEMU 以 exit 0 退出——只看退出码会假通过；verdict 用 `TEST_COMPLETE` 标记与
退出码对账，panic / oops 独立成档。

| verdict | 语义 |
|---|---|
| `passed` | 全部用例通过，标记协议走完 |
| `failed` | 有用例 FAILED（或汇总计数对不上） |
| `timeout` | 墙钟超时，进程组被收割 |
| `panic` | 内核 panic（独立成档；exit 可能是 0 或 124，见[故障排查](troubleshooting.md)） |
| `incomplete` | exit 0 但标记协议没走完——按失败处理，不得当作通过 |
| `interrupted` | Ctrl-C 中断收尾（工件已落盘；verdict.json 可能缺失） |
| `build_failed` | 构建阶段失败 |
| `unknown` | 未识别收尾形态 |

## 分析入口

```bash
cat target/runs/<id>/verdict.json        # 机读判定唯一面（八态 verdict + 逐测试明细）
virtuoso test --replay-until-fail 5      # flaky 返场：首个非 passed 即停
```

判定呈现不设第二命令：`test` 收尾打印 verdict 行；逐事件事实在
`events.jsonl`，人工细读直接看 run 目录下的 `serial.log` 与
`verdict.json`。Ctrl-C 中断的 run 工件已落盘，但 verdict.json 可能缺失
（标记协议未走完，人工判读按失败处理）。

症状速查见 [故障排查](troubleshooting.md)。
