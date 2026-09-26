//! 检查结果类型与呈现：Check/CheckKind/Level、Report（计数 + 渲染）、
//! msg label 剥离（detail）。

/// 检查主题（doctor 一屏分组的唯一依据；grouping 与 label 同源，无文本反解）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckKind {
    Config,
    HostTools,
    CrossCompile,
    KernelPath,
    KernelSource,
    KernelImage,
    KernelDocker,
    QemuBinary,
    QemuImg,
    Modules,
    Components,
    BusyBox,
    ToolsImage,
    Initrd,
}

impl CheckKind {
    /// 消息标签（msg 以 "<label>: " 开头；doctor 剥前缀取正文）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            CheckKind::Config => "Configuration",
            CheckKind::HostTools => "Host tools",
            CheckKind::CrossCompile => "Cross-compile",
            CheckKind::KernelPath => "KERNEL_PATH",
            CheckKind::KernelSource => "Kernel source",
            CheckKind::KernelImage => "Kernel image",
            CheckKind::KernelDocker => "Kernel docker",
            CheckKind::QemuBinary => "QEMU binary",
            CheckKind::QemuImg => "qemu-img",
            CheckKind::Modules => "Kernel modules",
            CheckKind::Components => "Components",
            CheckKind::BusyBox => "BusyBox",
            CheckKind::ToolsImage => "Tools image",
            CheckKind::Initrd => "Initrd",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Pass,
    Fail,
    Warn,
    Info,
}

#[derive(Debug)]
pub(crate) struct Check {
    pub level: Level,
    pub kind: CheckKind,
    pub msg: String,
    /// 一屏紧凑短语（Pass/Info 用；None = 不进一屏，如纯可选信息行）。
    pub summary: Option<String>,
}

/// msg 的 "<label>: " 前缀之后正文（Fail/Warn 逐行修复提示）。
pub(crate) fn detail(kind: CheckKind, msg: &str) -> &str {
    msg.strip_prefix(kind.label())
        .map(|r| r.trim_start_matches([':', ' ']))
        .unwrap_or(msg)
}

pub(super) fn check(
    level: Level,
    kind: CheckKind,
    msg: impl Into<String>,
    summary: Option<String>,
) -> Check {
    Check {
        level,
        kind,
        msg: msg.into(),
        summary,
    }
}
pub(super) fn pass(kind: CheckKind, msg: impl Into<String>, summary: Option<String>) -> Check {
    check(Level::Pass, kind, msg, summary)
}
pub(super) fn fail(kind: CheckKind, msg: impl Into<String>) -> Check {
    check(Level::Fail, kind, msg, None)
}
pub(super) fn warn(kind: CheckKind, msg: impl Into<String>) -> Check {
    check(Level::Warn, kind, msg, None)
}
pub(super) fn info(kind: CheckKind, msg: impl Into<String>, summary: Option<String>) -> Check {
    check(Level::Info, kind, msg, summary)
}

pub(crate) struct Report {
    pub checks: Vec<Check>,
    pub critical_pass: u32,
    pub critical_fail: u32,
    pub warnings: u32,
}

impl Report {
    /// 追加检查并重算计数（doctor 附加 docker 供给组用）。
    pub(crate) fn extend(&mut self, extra: Vec<Check>) {
        self.checks.extend(extra);
        self.recount();
    }

    fn recount(&mut self) {
        self.critical_pass = self
            .checks
            .iter()
            .filter(|c| c.level == Level::Pass)
            .count() as u32;
        self.critical_fail = self
            .checks
            .iter()
            .filter(|c| c.level == Level::Fail)
            .count() as u32;
        self.warnings = self
            .checks
            .iter()
            .filter(|c| c.level == Level::Warn)
            .count() as u32;
    }

    /// 渲染（tty 下着色；--verbose 全量清单）。
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&crate::ui::bold("[DOCTOR] QEMU E2E Prerequisites Check"));
        out.push_str("\n========================================\n");
        for chk in &self.checks {
            let (tag, code) = match chk.level {
                Level::Pass => ("[PASS]", "0;32"),
                Level::Fail => ("[FAIL]", "0;31"),
                Level::Warn => ("[WARN]", "0;33"),
                Level::Info => ("[INFO]", "0;36"),
            };
            out.push_str(&format!("  {} {}\n", crate::ui::paint(code, tag), chk.msg));
        }
        out
    }
}

/// 仅影响颜色，不影响判定（doctor 呈现层共用同一 NO_COLOR 语义）。
pub(crate) fn is_stdout_tty() -> bool {
    crate::ui::tty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_strips_kind_label() {
        assert_eq!(
            detail(
                CheckKind::QemuBinary,
                "QEMU binary: qemu-system-arm not found"
            ),
            "qemu-system-arm not found"
        );
        assert_eq!(detail(CheckKind::BusyBox, "BusyBox: cached (arm64)"), "cached (arm64)");
        // 未以 label 开头的消息原样保留（防御性）
        assert_eq!(detail(CheckKind::Config, "bare text"), "bare text");
    }
}
