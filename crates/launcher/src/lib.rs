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

/// POSIX shell 单参引用（shlex.quote 语义）：安全字符原样，其余整体单引号包裹。
/// 用于把 argv 渲染成可直接复制执行的一行启动命令。
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'@' | b'%' | b'+' | b','))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// `qemu-system-<arch> --version` 首行（verdict 运行指纹用）。
pub fn qemu_version(arch: Arch) -> Option<String> {
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
    use super::shell_quote;

    #[test]
    fn shell_quote_safe_chars_verbatim_spaces_quoted() {
        assert_eq!(shell_quote("virt"), "virt");
        assert_eq!(
            shell_quote("file=a.img,format=raw"),
            "file=a.img,format=raw"
        );
        assert_eq!(shell_quote("/tmp/a b.img"), "'/tmp/a b.img'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote(""), "''");
    }
}
