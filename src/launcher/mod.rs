//! launcher — 启动器：多架构矩阵与 QEMU 启动 DSL。
//!
//! `QemuInvocation` 强类型封装全部启动形态（KVM/TCG、NUMA、GDB stub、
//! 多 virtio-blk 数据盘、vfio-pci 透传），argv 冻结在原 `infra/run-qemu.sh`
//! （已删除的 shell 基线）上，由 qemu/argv.rs 的 `argv_*` 单测把守；
//! `QEMU=echo virtuoso shell` 可打印 argv 人工对照。架构矩阵定义在
//! crate::Arch —— builder（交叉前缀）与 cli 共同复用。
//!
//! 进程治理（原 guardian crate）并入 launcher 模块：启动与收割是一体的生命周期，
//! `guardian` 模块承载进程组 RAII、Ctrl-C 守护与墙钟看门狗。

pub(crate) mod guardian;
pub(crate) mod numa;
pub(crate) mod qemu;

use std::path::PathBuf;
use std::process::Command;

use crate::Arch;
pub(crate) use guardian::{install_ctrlc_guard, spawn_watchdog};
pub(crate) use numa::NumaTopology;
pub(crate) use qemu::{Accel, QemuInvocation};

/// 附加到 VM 的数据盘（rootfs 之外的 virtio-blk 块设备，如 tools.img）。
/// guest 内按 `-drive` 追加顺序映射为 /dev/vdb、/dev/vdc…（rootfs 恒为 /dev/vda）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataDisk {
    pub path: PathBuf,
}

impl DataDisk {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// 持久内存组件（DT 途径：arm64/riscv64 直启无 ACPI，QEMU nvdimm 的 NFIT
/// 需 EFI 引导才可见，故用 pmem-region 设备树节点绕开）。
/// - `size`：pmem 区域大小（"256M"），从 guest RAM 顶部挖出（2MiB 对齐）；
/// - `mem_limit`：cmdline 追加的 `mem=` 值（总内存 − pmem 区）——把 pmem 区
///   从内核线性内存模型中排除（等价 x86 `memmap=nn!ss` 语义）；
/// - `ram_backend`：主内存后端文件（memory-backend-file share=on），挖出的
///   pmem 区即宿主文件 backed —— guest 写入持久落盘、跨 run 保留；
/// - `dtb`：补丁后的设备树（cli 生成：dumpdtb + fdtput，根节点注入
///   pmem-region），经 `-dtb` 传入替换 QEMU 生成版。
///
/// guest 侧 of_pmem 注册 label-less nd_region → 免 ndctl 自动出现
/// /dev/pmem0（ZONE_DEVICE 映射，QUEUE_FLAG_DAX → fsdax 可挂载
/// `mount -o dax`）。x86_64 走 ACPI/e820 途径，另行支持。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmemSpec {
    pub size: String,
    pub mem_limit: String,
    pub ram_backend: PathBuf,
    pub dtb: PathBuf,
}

impl PmemSpec {
    pub(crate) fn new(
        size: impl Into<String>,
        mem_limit: impl Into<String>,
        ram_backend: impl Into<PathBuf>,
        dtb: impl Into<PathBuf>,
    ) -> Self {
        Self {
            size: size.into(),
            mem_limit: mem_limit.into(),
            ram_backend: ram_backend.into(),
            dtb: dtb.into(),
        }
    }
}

/// `qemu-system-<arch> --version` 首行（verdict 运行指纹用）。
pub(crate) fn qemu_version(arch: Arch) -> Option<String> {
    let out = Command::new(arch.qemu_bin())
        .arg("--version")
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shell_quote_safe_chars_verbatim_spaces_quoted() {
        // 语义钉在 crate::shell（本 crate 经 re-export 消费）
        assert_eq!(crate::util::quote("/tmp/a b.img"), "'/tmp/a b.img'");
        assert_eq!(crate::util::quote("a'b"), "'a'\\''b'");
    }
}
