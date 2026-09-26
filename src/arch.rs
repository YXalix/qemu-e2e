//! 目标架构矩阵与宿主平台 —— 原 3 个 shell 脚本里散落表格的唯一事实来源。

/// 目标架构。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arch {
    Arm64,
    X86_64,
    Riscv64,
}

impl Arch {
    /// 与 verify.sh / run-qemu.sh 相同的别名归一逻辑。
    pub(crate) fn parse(s: &str) -> Option<Arch> {
        match s {
            "aarch64" | "arm64" => Some(Arch::Arm64),
            "x86_64" | "amd64" => Some(Arch::X86_64),
            "riscv64" => Some(Arch::Riscv64),
            _ => None,
        }
    }

    /// 宿主机架构（脚本中的 `uname -m` 等价物）。
    pub(crate) fn host_default() -> Option<Arch> {
        Self::parse(std::env::consts::ARCH)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X86_64 => "x86_64",
            Arch::Riscv64 => "riscv64",
        }
    }

    pub(crate) fn qemu_bin(self) -> &'static str {
        match self {
            Arch::Arm64 => "qemu-system-aarch64",
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::Riscv64 => "qemu-system-riscv64",
        }
    }

    /// 相对内核树的镜像路径。
    pub(crate) fn kernel_img(self) -> &'static str {
        match self {
            Arch::Arm64 => "arch/arm64/boot/Image",
            Arch::X86_64 => "arch/x86/boot/bzImage",
            Arch::Riscv64 => "arch/riscv/boot/Image",
        }
    }

    pub(crate) fn console(self) -> &'static str {
        match self {
            Arch::Arm64 => "ttyAMA0",
            Arch::X86_64 | Arch::Riscv64 => "ttyS0",
        }
    }

    /// machine 类型。**冻结基线（原 run-qemu.sh 行为）：三架构一律 `virt`**
    /// （脚本从未使用 q35；此处保留单一入口，将来引入 q35 只改这里）。
    #[allow(unused_variables)]
    pub(crate) fn machine(self) -> &'static str {
        "virt"
    }

    /// TCG 模式下的 CPU 型号（run-qemu.sh 的 CPU_TCG 列）。
    pub(crate) fn cpu_tcg(self) -> &'static str {
        match self {
            Arch::Arm64 => "cortex-a72",
            Arch::X86_64 => "qemu64",
            Arch::Riscv64 => "rv64",
        }
    }

    /// Rust 静态 musl target triple（tools workspace 构建用；crt-static 自含
    /// 链接，产物为无动态依赖的静态 ELF —— VM 无动态加载器的对齐选择）。
    pub(crate) fn rust_musl_triple(self) -> &'static str {
        match self {
            Arch::Arm64 => "aarch64-unknown-linux-musl",
            Arch::X86_64 => "x86_64-unknown-linux-musl",
            Arch::Riscv64 => "riscv64gc-unknown-linux-musl",
        }
    }

    /// `zig cc -target` triple（非 Linux 宿主交叉编译；zig 自带 musl
    /// sysroot，静态由 musl 目标天然满足）。
    pub(crate) fn zig_triple(self) -> &'static str {
        match self {
            Arch::Arm64 => "aarch64-linux-musl",
            Arch::X86_64 => "x86_64-linux-musl",
            Arch::Riscv64 => "riscv64-linux-musl",
        }
    }

}

// ---------------------------------------------------------------- 宿主平台

// 宿主平台 —— QEMU 参数平台分支的唯一事实来源。
//
// 项目面向 Linux 与 macOS（Apple Silicon：QEMU 走 HVF 加速，且无 memfd
// 内存后端）。argv 冻结基线按宿主平台各持一份：launcher 的 `argv_*` 单测
// 显式钉死 `HostOs`，宿主平台只影响缺省取值，不影响冻结文本。

/// 宿主操作系统（QEMU 能力差异的最小分类面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostOs {
    Linux,
    Darwin,
}

impl HostOs {
    /// 编译期宿主平台；未识别平台按 Linux 语义兜底。
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            HostOs::Darwin
        } else {
            HostOs::Linux
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            HostOs::Linux => "linux",
            HostOs::Darwin => "darwin",
        }
    }
}

#[cfg(test)]
mod host_os_tests {
    use super::HostOs;

    #[test]
    fn current_matches_compile_target() {
        // 本 crate 零依赖也不做交叉编译，current() 只由编译目标决定
        let expect = if cfg!(target_os = "macos") {
            HostOs::Darwin
        } else {
            HostOs::Linux
        };
        assert_eq!(HostOs::current(), expect);
    }
}
