# tools_disk — VM 内工具盘

把 `infra/tools/` workspace 构建的常驻工具装进独立数据盘，随 VM 外挂，
与测试用例（`/tests/`，参与判定）正交分离。

## 配置

```toml
[components.tools_disk]   # 段缺省 = 启用；显式关闭：
# enabled = false
```

公共字段 `enabled` / `require` / `stage` 均可用；缺省 `require` 为空
（virtio_blk / ext4 已在 boot 冻结基础集里）。

## 工作方式

1. builder 构建 `infra/tools/`（std Rust + **musl 静态**）→ 装入 ext4 数据盘
   `/bin/`（卷标 `tools`）→ `target/artifacts/tools.img`；
2. 启动时作为 rootfs 之后的第一个 virtio-blk 数据盘附加（guest 内
   `/dev/vdb`；rootfs 恒 `/dev/vda`）；
3. rootfs 的 `/init-hooks.sh`（builder 生成）把 `/dev/vdb` 挂到 `/tools` 并把
   `/tools/bin` 注入 `PATH`——位于 devtmpfs 挂载后、insmod / agent 拉起前。

工具不进 `/tests/`、不参与判定：它们是运行环境的一部分（如 `virtuoso-agent`），
不是被测对象。

## 装什么

`infra/tools/` 是独立 workspace，与 testcases 分类正交：

- 当前成员：`agent/` = virtuoso-agent（[agent 组件](agent.md)的 guest 侧）；
- 新工具 = 新 crate，musl 静态产物自动进 tools.img 的 `/bin/`。

## 降级行为

工具供给缺失（如宿主缺 musl target）时**显式降级非掩盖**：不产出 tools.img、
不注入挂载 hook，构建日志给出原因。`enabled = false` 时同理会跳过整条链路。
