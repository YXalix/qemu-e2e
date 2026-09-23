//! 宿主平台 —— QEMU 参数平台分支的唯一事实来源。
//!
//! 项目面向 Linux 与 macOS（Apple Silicon：QEMU 走 HVF 加速，且无 memfd
//! 内存后端）。argv 冻结基线按宿主平台各持一份：launcher 的 `argv_*` 单测
//! 显式钉死 `HostOs`，宿主平台只影响缺省取值，不影响冻结文本。

/// 宿主操作系统（QEMU 能力差异的最小分类面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    Darwin,
}

impl HostOs {
    /// 编译期宿主平台；未识别平台按 Linux 语义兜底。
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            HostOs::Darwin
        } else {
            HostOs::Linux
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HostOs::Linux => "linux",
            HostOs::Darwin => "darwin",
        }
    }
}

#[cfg(test)]
mod tests {
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
