# vfio — PCI 直通

把宿主物理 PCI 设备直通给 guest：每个 BDF 生成一条
`-device vfio-pci,host=<bdf>`。

## 配置

```toml
[components.vfio]
enabled = true
devices = ["0000:01:00.0"]        # 宿主设备的 BDF 列表（lspci 查）
require = ["vfio", "vfio_pci"]    # guest 内核 =m 时声明
stage = "boot"                    # root 挂载前就要 → boot（缺省 runtime）
```

## 前置要求

- **宿主开 IOMMU**：x86_64 内核参数 `intel_iommu=on`（或 `amd_iommu`）；
  arm64 需要 SMMU 平台。
- 设备已绑定宿主 `vfio-pci` 驱动（`driverctl` 或手动解绑再绑）。
- guest 内核侧 `vfio` / `vfio_pci` 可用：`=y` 无需声明，`=m` 时写进 `require`
  （直通设备若要在 root 挂载前就绪，加 `stage = "boot"`）。

## 行为

- `devices` 逐条生成 `-device vfio-pci,host=<bdf>`，追加在 argv 冻结基线之后；
