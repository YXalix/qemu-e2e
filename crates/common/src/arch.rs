//! 目标架构矩阵 —— 原 3 个 shell 脚本里散落表格的唯一事实来源。

/// 目标架构。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X86_64,
    Riscv64,
}

pub const ALL_ARCHES: [Arch; 3] = [Arch::Arm64, Arch::X86_64, Arch::Riscv64];

impl Arch {
    /// 与 verify.sh / run-qemu.sh 相同的别名归一逻辑。
    pub fn parse(s: &str) -> Option<Arch> {
        match s {
            "aarch64" | "arm64" => Some(Arch::Arm64),
            "x86_64" | "amd64" => Some(Arch::X86_64),
            "riscv64" => Some(Arch::Riscv64),
            _ => None,
        }
    }

    /// 宿主机架构（脚本中的 `uname -m` 等价物）。
    pub fn host_default() -> Option<Arch> {
        Self::parse(std::env::consts::ARCH)
    }

    pub fn name(self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X86_64 => "x86_64",
            Arch::Riscv64 => "riscv64",
        }
    }

    pub fn qemu_bin(self) -> &'static str {
        match self {
            Arch::Arm64 => "qemu-system-aarch64",
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::Riscv64 => "qemu-system-riscv64",
        }
    }

    /// 相对内核树的镜像路径。
    pub fn kernel_img(self) -> &'static str {
        match self {
            Arch::Arm64 => "arch/arm64/boot/Image",
            Arch::X86_64 => "arch/x86/boot/bzImage",
            Arch::Riscv64 => "arch/riscv/boot/Image",
        }
    }

    pub fn console(self) -> &'static str {
        match self {
            Arch::Arm64 => "ttyAMA0",
            Arch::X86_64 | Arch::Riscv64 => "ttyS0",
        }
    }

    /// machine 类型。**以 run-qemu.sh 的真实行为为准：三架构一律 `virt`**
    /// （脚本从未使用 q35；此处保留单一入口，将来引入 q35 只改这里）。
    pub fn machine(self) -> &'static str {
        let _ = self;
        "virt"
    }

    /// TCG 模式下的 CPU 型号（run-qemu.sh 的 CPU_TCG 列）。
    pub fn cpu_tcg(self) -> &'static str {
        match self {
            Arch::Arm64 => "cortex-a72",
            Arch::X86_64 => "qemu64",
            Arch::Riscv64 => "rv64",
        }
    }

    /// 交叉编译前缀（builder 编译 C 用例用）；本机架构返回 None。
    pub fn cross_prefix(self) -> Option<&'static str> {
        match self {
            Arch::Arm64 => Some("aarch64-linux-gnu-"),
            Arch::Riscv64 => Some("riscv64-linux-gnu-"),
            Arch::X86_64 => None,
        }
    }
}
