# 运行工件与跨 run 分析

每次 `virtuoso test` / `matrix` 写入 `target/runs/<unix_ms>-<arch>/`（保留最近
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

**终端呈现同样按流分离**（test / probe / matrix）：运行中终端只回显 guest
串口（即 `serial.log` 的内容）；QEMU 自身输出不刷屏，只落 `qemu-stderr.log`，
失败或超时收尾时回显其末尾至多 40 行（QEMU 早夭、参数被拒的现场就在这几行
里），成功保持安静。`virtuoso shell` / `debug` 是交互式会话，stdio 天然合流，
QEMU 输出仍实时可见。

## Verdict 语义

`judge::Verdict` 八态：`passed` / `failed` / `timeout` / `panic` /
`incomplete` / `interrupted` / `build_failed` / `unknown`。

**判定以 verdict 为准，退出码只是接口契约**：`-no-reboot` 下内核 panic 会让
QEMU 以 exit 0 退出——只看退出码会假通过；verdict 用 `TEST_COMPLETE` 标记与
退出码对账，panic / oops 独立成档。`exit 0` 但 `verdict: incomplete` = 标记
协议没走完，按失败处理。

## 分析命令

```bash
virtuoso triage [--run <id>] [--json]    # 最近（或指定）run 的分诊报告
virtuoso runs [--json]                   # 历史运行列表（最新在前）
virtuoso matrix [--arch a]               # 多架构矩阵（缺省三架构，串行）
virtuoso test --replay-until-fail 5      # flaky 返场：首个非 passed 即停
virtuoso cluster [--json]                # 跨 run 失败指纹聚类 + flaky 清单
virtuoso suggest [--diff f]              # git diff → 推荐最小测试集
virtuoso replay --log <f>                # 任意串口日志的离线标记协议断言
```

`triage` / `runs` / `cluster` / `suggest` / `replay` 都支持 `--json`，可直接
进管道。语义细节：

- **cluster**：失败指纹 = verdict 类 + 归一化证据（剥离内核时间戳、数字折叠为
  `N`；panic/oops 优先 → timeout → 失败测试集）；flaky 判定只信
  passed/failed 的 run，并给出每类失败的首现 run；
- **suggest**：git diff 的子系统路径前缀 → 最小测试集（`--diff` 缺省对
  KERNEL_PATH 内核树做 git diff，含暂存区）；
- Ctrl-C 中断的 run：工件已落盘，`triage` 对缺失的 verdict.json 自动降级为
  现场解析 serial.log（`verdict: unknown`）。

症状速查见 [Troubleshooting](../troubleshooting.md)。
