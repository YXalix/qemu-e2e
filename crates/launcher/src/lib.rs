//! launcher — 启动器：多架构矩阵与 QEMU/Firecracker 启动 DSL。
//!
//! `QemuInvocation` 强类型封装 run-qemu.sh 的全部启动形态（KVM/TCG、NUMA、
//! GDB stub、NVMe、vfio-pci 透传），argv 顺序与脚本逐字对齐（`QEMU=echo
//! infra/run-qemu.sh` 可抽取脚本真实 argv 做对照）。架构矩阵定义在 common::Arch
//! —— builder（交叉前缀）与 xtask 共同复用。Firecracker 后端见 `firecracker`。

pub mod firecracker;
pub mod numa;
pub mod qemu;

use std::process::Command;

pub use common::{Arch, ALL_ARCHES};
pub use numa::NumaTopology;
pub use qemu::{Accel, QemuInvocation};

/// 启动后端（Phase 3）：QEMU（缺省，全形态）与 Firecracker（microVM，
/// x86_64/aarch64 + KVM，见 `firecracker` 模块）。判定协议对后端不感知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    Qemu,
    Firecracker,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Backend> {
        match s.trim() {
            "qemu" | "QEMU" => Some(Backend::Qemu),
            "firecracker" | "fc" => Some(Backend::Firecracker),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Qemu => "qemu",
            Backend::Firecracker => "firecracker",
        }
    }
}

/// `qemu-system-<arch> --version` 首行（verdict 运行指纹用）。
pub fn qemu_version(arch: Arch) -> Option<String> {
    let out = Command::new(arch.qemu_bin()).arg("--version").output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next().map(str::to_string)
}
