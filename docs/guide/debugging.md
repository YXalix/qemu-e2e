# 调试

## 交互 shell

```bash
virtuoso shell            # TCG（跨架构适用）
virtuoso shell --kvm      # 原生加速（宿主 = 目标架构时）
```

退出：`Ctrl-A` 然后 `x`。rootfs 里保留全部 `/tests/*` 用例，可手动复跑单个
测试复现失败。

## GDB 源码级内核调试

内核开 `CONFIG_DEBUG_INFO=y`（建议加 `CONFIG_DEBUG_INFO_DWARF5=y`、
`CONFIG_GDB_SCRIPTS=y`）。

```bash
virtuoso debug            # 终端 1：挂起启动，监听 :1234
cd "$KERNEL_PATH" && \    # 终端 2：
gdb-multiarch vmlinux -ex 'target remote :1234'
```

## AI probe（virtio-serial 通道）

```bash
virtuoso probe --cmd 'uname -a' --cmd 'dmesg | tail'
virtuoso probe --cmd-file cmds.txt --json    # 机器可读事件流
```

经 guest 侧 virtuoso-agent（JSON 行协议）下发命令批，事件流写 run 目录的
`agent-events.jsonl`。通道详解见 [agent 组件](../components/agent.md)，
AI 工作流见 [AI 集成](ai-integration.md)。

## Firecracker 后端

```bash
virtuoso doctor --backend firecracker    # 一屏体检（含 microVM preflight 组）
virtuoso verify --backend firecracker    # 全量清单 + microVM preflight
virtuoso test --timeout 60 --backend firecracker
```

x86_64 / aarch64 + KVM；不支持 agent 通道与 pmem（WARN 忽略），numa 拓扑
扁平化——见[后端支持矩阵](../components/overview.md#后端支持矩阵)。

## argv 人工复核

```bash
QEMU=echo virtuoso shell    # 只打印 QEMU argv，不启动
```

## 离线判定回放

```bash
virtuoso replay --log <serial.log>    # 对任意串口日志做标记协议断言，不启动 QEMU
```
