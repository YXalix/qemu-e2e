# pmem — 持久内存

guest 内的持久内存区域：`/dev/pmem0` + DAX，写入对宿主文件持久落盘。
**仅 arm64 / riscv64**（走设备树途径）。

## 配置

```toml
[components.pmem]                  # 段缺省 = 关闭
enabled = true
size = "256M"                      # 从 guest RAM 顶部挖出（须小于总内存）
require = ["libnvdimm", "nd_btt", "of_pmem", "nd_pmem"]   # 内核 =m 时声明（含顺序）
```

`require` 是实测最小集：`nd_btt` 对 `libnvdimm` 是硬依赖，顺序错或缺失会
insmod 失败。内核把这些选项编成 `=y` 时无需声明。

## 工作方式（DT 途径三件套）

QEMU 的 NFIT/NVDIMM 途径需要 EFI，openEuler 的 QEMU 又没有 virtio-pmem——
所以走**设备树**：launcher 在启动前生成三件套（落 `target/build/pmem/`）：

1. **dumpdtb** 导出 QEMU 的原生设备树，`fdtput` 注入 `pmem-region` 节点
   （of_pmem 绑定），启动时经 `-dtb` 回写；
2. **cmdline 追加 `mem=<总内存 − pmem 区>`**：把区间排除出内核线性内存
   模型，`devm_memremap_pages` 才能建 ZONE_DEVICE（不排除会 no-map 冲突
   EEXIST）；
3. **主内存后端换宿主文件**（memory-backend-file）：guest 对该区间的写入
   由 KVM 直接落到宿主文件，即持久化。

guest 内：`/dev/pmem0` 出现后即可 mkfs / dax 挂载，重跑 VM 数据仍在。

## 限制

- 仅 arm64 / riscv64（DT 途径）；x86_64 无此途径。
- `size` 必须小于总内存，否则解析期拒绝。
