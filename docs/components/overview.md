# 组件机制

VM 能力按组件配置在 `virtuoso.toml` 的 `[components.*]` 段。组件是
**声明式的**：开关 + KO 依赖，builder 负责把启用组件的需求并成模块清单、
launcher 负责生成对应的 QEMU 参数。

## 组件一览

| 组件 | 作用 | 段缺省 | 专属键 |
|---|---|---|---|
| [`tools_disk`](tools-disk.md) | tools.img 数据盘 → `/dev/vdb` 挂 `/tools`（VM 内工具，musl 静态） | **启用** | — |
| [`agent`](agent.md) | AI probe 通道（virtio-serial + guest 侧 virtuoso-agent） | 关闭 | — |
| [`vfio`](vfio.md) | PCI 直通，逐条生成 `-device vfio-pci,host=<bdf>` | 关闭 | `devices` |
| [`numa`](numa.md) | 多节点拓扑（每节点一个 socket） | 关闭 | `nodes`、`memory_per_node` |
| [`pmem`](pmem.md) | 持久内存（DT 途径 → `/dev/pmem0` + DAX） | 关闭 | `size`、`require` |

`[busybox]` 是全局段（非组件），`[tests]` 是测例套件段（收用例需要的
模块依赖），见[配置参考](../guide/configuration.md)。

## 公共字段

每个 `[components.<name>]` 段支持：

- `enabled`：开关。**段缺省 = 各组件自己的缺省**（tools_disk 缺省启用，其余缺省关闭）。
- `require`：KO 依赖，条目 = conf 行 `"<module> [key=val ...]"`（token 原样透传 insmod）。
- `stage`：`boot`（root 挂载前就要）｜ `runtime`（缺省）。

```toml
[components.agent]          # 段缺省 = 关闭；取消注释即启用
enabled = true
require = ["virtio_console"]
```

仓库根的 `virtuoso.toml` 模板中，可选组件全部以注释形式在场，取消注释即启用。

## 模块清单由组件生成（不手写）

模块供给是组件机制的直接产物：

- `infra/modules-boot.conf`（**冻结基础集**：virtio + ext4 及依赖）进 initramfs，
  由 `init-initramfs` 在 pivot 前 insmod；
- 组件条目 `stage = "boot"` 追加到基础集之后；
- 其余来自启用组件的 `require` **并集**（外加 `[tests]` 段的测例模块依赖，
  排在最后），builder 写入 rootfs `/lib/modules/modules.conf`，测试 init 在
  pivot 后加载。

并集规则：schema 固定顺序 tools_disk→agent→vfio→numa→pmem→`[tests]`，
按首 token 去重保首个。

规则与陷阱：

- **顺序敏感**：按清单顺序 insmod（无自动依赖解析），依赖在前。
- **模块必须已构建**：`kernel_path` 下找不到对应 `.ko` 直接构建报错。
- **迭代中的模块优先 `=m` 而非 `=y`**：免内核重建，回路更快。
- 判断 `stage` 的标准：**这个模块是不是"root 挂上之前就必须在内核里"**？
  是 → `stage = "boot"`，否 → 缺省 runtime。

`virtuoso probe` 恒开 agent 通道（强制并入 `virtio_console`，不依赖组件开关）。
