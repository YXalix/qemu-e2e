# 冻结契约

以下接口对外冻结：改动任何一条都会破坏下游（CI、AI skill、脚本管道）的解析，
演进必须走新版本协议，不许原地改语义。

| 维度 | 契约 |
|---|---|
| 串口标记协议 | v1 冻结（附录）：改动文本等于破坏所有下游解析 |
| 退出码 | 0=通过、124=超时（137 归一）、其余=失败；唯一表在 `judge::exit` |
| QEMU argv | 缺省（无数据盘、无 agent）输出与**宿主平台各自的冻结基线**逐字一致，`argv_*` 单测把守 |
| 静态链接 | 测试必须 `-static`；禁止 `\|\| true` 掩盖失败 |
| 配置优先级 | 标量键：进程环境变量 > `virtuoso.toml`（同名键 env 覆盖） |
| AI 接口 | skill 只依赖标记协议 v1 与工件 schema（verdict.json / events.jsonl），不依赖 harness 内部实现 |

## 退出码语义

| 退出码 | 含义 |
|---|---|
| `0` | 通过（且 verdict 必须是 `passed` 才算真通过） |
| `124` | 墙钟超时（QEMU 被 KILL；137 归一为 124） |
| `130` | Ctrl-C 中断（工件已落盘） |
| 其余 | 失败 |

注意：`-no-reboot` 下内核 panic 会让 QEMU 以 **exit 0** 退出——退出码只是
接口契约，判定以 `verdict.json` 为准。

## argv 冻结基线

`QemuInvocation::argv` 在缺省形态（无数据盘、无 agent 通道）下的输出与既定
基线**逐字一致**，由 `crates/launcher/src/qemu.rs` 的 `argv_*` 单测把守。
**基线按宿主平台各持一份**（`common::HostOs`，单测显式钉死，不随编译目标
漂移）：两平台的差异面收敛在两处——

| 段 | Linux | macOS（Darwin） |
|---|---|---|
| 内存后端 | `memory-backend-memfd,…,share=off` | `memory-backend-ram,…`（QEMU 无 memfd） |
| 硬件加速 | `shell --kvm` → `-enable-kvm`；test 恒 TCG（无 accel flag） | 同构 guest 缺省/`--tcg` 之外的路径 → `-accel hvf`；`--tcg` 强制纯模拟；accel × 平台错配（KVM@mac、HVF@linux）在 argv 构造期报错 |

数据盘（tools.img 等）与 agent 串口属调用方增量，追加在基线之后。人工复核：

```bash
QEMU=echo virtuoso shell    # 只打印 argv，不启动 QEMU
```

## 附录：串口标记协议 v1（冻结文本）

```text
# 测试二进制汇编层（infra/init 打印）
--- Running: <name> ---
PASSED: <name>
FAILED: <name>

# 单断言行（测试二进制内打印）
[PASS] <message>
[FAIL] <message>
[SKIP] <message>
[INFO] <message>

# 单二进制汇总（init 依各用例退出码汇编）
Test Results: <N>/<M> passed

# 全局终止标记（harness 判定依据）
TEST_COMPLETE: ALL TESTS PASSED     # → exit 0
TEST_COMPLETE: SOME TESTS FAILED    # → exit 1

# 超时（宿主侧判定）
exit 124   # wallclock timeout（内核挂死 / runaway loop）
```

judge 对以上标记逐行解析为结构化事件（见 `crates/judge/src/lib.rs`）。
用例侧只依赖 `[PASS]` 等断言宏，汇编层标记由 `infra/init` 输出。编写测试见
[编写测试用例](../guide/writing-tests.md)。
