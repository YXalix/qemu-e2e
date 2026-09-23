# numa — 多节点拓扑

多节点 NUMA 拓扑：每节点一个 socket，guest 内可见真实的节点/内存 locality。

## 配置

```toml
smp = 8                            # 全局键：vCPU 总数

[components.numa]                  # 段缺省（或 enabled=false）= 单节点
enabled = true
nodes = 2
memory_per_node = "1G"
```

## 约束（解析期校验，启动期拒绝）

- `smp` 必须被 `nodes` **整除**（解析期报错；launcher 二次校验）；
- 单节点（组件关闭）时不传 `-numa`，argv 保持冻结基线；
- 总内存 = `nodes × memory_per_node`。

## 行为

launcher 为每个节点生成一个 socket（CPU 与 memory-backend 一一对应），
`-smp` 取全局值。交叉架构 / 无 KVM 时照常工作（TCG 也支持 NUMA 拓扑）。

Firecracker 后端下拓扑被**扁平化**：折算为 `vcpu_count`（= smp）+
`mem_size_mib`（= 总内存），无多节点语义。
