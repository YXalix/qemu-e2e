//! 加速器选择：KVM（Linux）/ HVF（macOS）仅宿主与目标同构时可用
//! （argv 装配期校验），交叉架构回退 TCG。

use crate::{Arch, HostOs};

/// 加速器：KVM（Linux）/ HVF（macOS）仅宿主与目标同构时可用（调用方校验），
/// 交叉架构回退 TCG。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accel {
    Tcg,
    Kvm,
    Hvf,
}

impl Accel {
    /// verdict.json 运行指纹里的呈现名。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Accel::Tcg => "TCG",
            Accel::Kvm => "KVM",
            Accel::Hvf => "HVF",
        }
    }

    /// 平台缺省加速器：macOS 上宿主架构与目标同构 → HVF（Apple Silicon 近
    /// 原生），其余（含 Linux 全部场景）→ TCG。Linux 的 KVM 仍由 `shell
    /// --kvm` 显式开启；macOS 想强制纯模拟用 `--tcg`。宿主架构显式传参
    /// （运行时取 `Arch::host_default()`）而非函数内自查——编译目标推断会让
    /// 钉死他平台行为的单测在 CI 的另一架构上必红。
    pub(crate) fn default_for(arch: Arch, host: HostOs, host_arch: Option<Arch>) -> Accel {
        if host == HostOs::Darwin && host_arch == Some(arch) {
            Accel::Hvf
        } else {
            Accel::Tcg
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_accel_only_hvf_on_darwin_same_arch() {
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Darwin, Some(Arch::Arm64)),
            Accel::Hvf,
            "Apple Silicon 宿主跑 arm64 guest 缺省 HVF"
        );
        assert_eq!(
            Accel::default_for(Arch::X86_64, HostOs::Darwin, Some(Arch::Arm64)),
            Accel::Tcg,
            "Apple Silicon 宿主交叉 x86_64 guest 回落 TCG"
        );
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Darwin, None),
            Accel::Tcg,
            "宿主架构未知回落 TCG"
        );
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Linux, Some(Arch::Arm64)),
            Accel::Tcg,
            "Linux 全场景缺省 TCG（KVM 仍由 --kvm 显式开启）"
        );
    }
}
