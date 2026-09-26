//! doctor 一屏呈现（flutter-doctor 风格）：引擎检查按 CheckKind 归并为
//! 组件组行，✓/✗/! 一眼可读。分组唯一依据是 CheckKind（引擎自带），无
//! 消息文本反解；Pass/Info 行取引擎预计算的 summary 紧凑短语，Fail/Warn
//! 行取 label 剥离后的全文。

use crate::builder::verify::{detail, CheckKind, Level, Report};

// ---------------------------------------------------------------- 引擎检查 → 组件分组

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Config,
    Toolchain,
    Kernel,
    Qemu,
    Modules,
    Artifacts,
}

impl Group {
    pub(super) fn name(self) -> &'static str {
        match self {
            Group::Config => "Config",
            Group::Toolchain => "Toolchain",
            Group::Kernel => "Kernel",
            Group::Qemu => "QEMU",
            Group::Modules => "Modules",
            Group::Artifacts => "Artifacts",
        }
    }
}

/// 固定呈现顺序。
const GROUP_ORDER: &[Group] = &[
    Group::Config,
    Group::Toolchain,
    Group::Kernel,
    Group::Qemu,
    Group::Modules,
    Group::Artifacts,
];

/// 引擎检查主题 → 一屏分组（穷尽匹配：引擎新增 kind 时此处编译期报错）。
fn group_of(kind: CheckKind) -> Group {
    match kind {
        CheckKind::Config => Group::Config,
        CheckKind::HostTools | CheckKind::CrossCompile => Group::Toolchain,
        CheckKind::KernelPath
        | CheckKind::KernelSource
        | CheckKind::KernelImage
        | CheckKind::KernelDocker => Group::Kernel,
        CheckKind::QemuBinary | CheckKind::QemuImg => Group::Qemu,
        CheckKind::Modules | CheckKind::Components => Group::Modules,
        CheckKind::BusyBox | CheckKind::ToolsImage | CheckKind::Initrd => Group::Artifacts,
    }
}

/// 组内最严重级别（Info 视同 Pass：可选工具缺失不降级）。
fn worst(a: Level, b: Level) -> Level {
    let rank = |l: Level| match l {
        Level::Fail => 3,
        Level::Warn => 2,
        Level::Pass => 1,
        Level::Info => 0,
    };
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------- 组装与呈现

#[derive(Debug, PartialEq)]
pub(super) struct GroupOut {
    pub(super) group: Group,
    pub(super) level: Level,
    /// (级别, 文本)：Pass/Info 为紧凑短语（合并渲染），Fail/Warn 为全文（逐行）。
    pub(super) lines: Vec<(Level, String)>,
}

fn slot(out: &mut [GroupOut], g: Group) -> &mut GroupOut {
    out.iter_mut()
        .find(|s| s.group == g)
        .expect("GROUP_ORDER 覆盖全部分组")
}

pub(super) fn group_checks(report: &Report) -> Vec<GroupOut> {
    let mut out: Vec<GroupOut> = GROUP_ORDER
        .iter()
        .map(|g| GroupOut {
            group: *g,
            level: Level::Info,
            lines: Vec::new(),
        })
        .collect();
    for chk in &report.checks {
        let s = slot(&mut out, group_of(chk.kind));
        s.level = worst(s.level, chk.level);
        match chk.level {
            Level::Pass | Level::Info => {
                if let Some(short) = &chk.summary {
                    if !s.lines.iter().any(|(_, l)| l == short) {
                        s.lines.push((chk.level, short.clone()));
                    }
                }
            }
            Level::Fail | Level::Warn => {
                s.lines.push((
                    chk.level,
                    detail(chk.kind, &chk.msg).to_string(),
                ));
            }
        }
    }
    out.retain(|s| !s.lines.is_empty());
    out
}

pub(super) fn render(groups: &[GroupOut], tty: bool) -> String {
    let c = |code: &str, s: &str| {
        if tty {
            crate::ui::paint(code, s)
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    for g in groups {
        let (icon, code) = match g.level {
            Level::Fail => ("✗", "0;31"),
            Level::Warn => ("!", "0;33"),
            _ => ("✓", "0;32"),
        };
        let mut rows: Vec<String> = Vec::new();
        let bad: Vec<&String> = g
            .lines
            .iter()
            .filter(|(l, _)| matches!(l, Level::Fail | Level::Warn))
            .map(|(_, t)| t)
            .collect();
        let good = g
            .lines
            .iter()
            .filter(|(l, _)| matches!(l, Level::Pass | Level::Info))
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        if !bad.is_empty() {
            rows.extend(bad.iter().map(|s| (*s).clone()));
            if !good.is_empty() {
                rows.push(good);
            }
        } else {
            rows.push(good);
        }
        let name = format!("{:<11}", g.group.name());
        out.push_str(&format!(
            "  {} {}  {}\n",
            c(code, icon),
            c("1", name.as_str()),
            rows[0]
        ));
        for row in &rows[1..] {
            // 与首行正文列对齐：2 缩进 + 图标 1 + 空格 + 组名 11 + 2 空格
            out.push_str(&format!("{:<17}{}\n", "", row));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::verify::Check;

    fn check(level: Level, kind: CheckKind, msg: &str, summary: Option<&str>) -> Check {
        Check {
            level,
            kind,
            msg: msg.into(),
            summary: summary.map(str::to_string),
        }
    }

    fn report(checks: Vec<Check>) -> Report {
        let critical_fail = checks.iter().filter(|c| c.level == Level::Fail).count() as u32;
        let warnings = checks.iter().filter(|c| c.level == Level::Warn).count() as u32;
        Report {
            checks,
            critical_pass: 0,
            critical_fail,
            warnings,
        }
    }

    #[test]
    fn group_of_covers_every_kind() {
        // 穷尽性由 group_of 的 match 保证（新增 kind 编译期报错）；
        // 这里钉死每个 kind 的预期分组，防止无意挪组破坏一屏布局。
        for (kind, want) in [
            (CheckKind::Config, Group::Config),
            (CheckKind::HostTools, Group::Toolchain),
            (CheckKind::CrossCompile, Group::Toolchain),
            (CheckKind::KernelPath, Group::Kernel),
            (CheckKind::KernelSource, Group::Kernel),
            (CheckKind::KernelImage, Group::Kernel),
            (CheckKind::KernelDocker, Group::Kernel),
            (CheckKind::QemuBinary, Group::Qemu),
            (CheckKind::QemuImg, Group::Qemu),
            (CheckKind::Modules, Group::Modules),
            (CheckKind::Components, Group::Modules),
            (CheckKind::BusyBox, Group::Artifacts),
            (CheckKind::ToolsImage, Group::Artifacts),
            (CheckKind::Initrd, Group::Artifacts),
        ] {
            assert_eq!(group_of(kind), want, "kind={kind:?}");
        }
    }

    #[test]
    fn worst_orders_fail_over_warn_over_pass() {
        assert_eq!(worst(Level::Info, Level::Pass), Level::Pass);
        assert_eq!(worst(Level::Pass, Level::Warn), Level::Warn);
        assert_eq!(worst(Level::Warn, Level::Fail), Level::Fail);
    }

    #[test]
    fn group_checks_fails_lead_and_warn_keeps_full_text() {
        let groups = group_checks(&report(vec![
            check(
                Level::Pass,
                CheckKind::Config,
                "Configuration: virtuoso.toml found",
                Some("virtuoso.toml found"),
            ),
            check(
                Level::Fail,
                CheckKind::QemuBinary,
                "QEMU binary: qemu-system-arm not found (install qemu-system-arm)",
                None,
            ),
            check(
                Level::Warn,
                CheckKind::Modules,
                "Kernel modules: 2/3 found, missing: nd_btt",
                None,
            ),
            check(
                Level::Pass,
                CheckKind::KernelImage,
                "Kernel image: Image (42M)",
                Some("Image (42M)"),
            ),
        ]));
        let qemu = groups.iter().find(|g| g.group == Group::Qemu).unwrap();
        assert_eq!(qemu.level, Level::Fail);
        assert_eq!(
            qemu.lines,
            vec![(
                Level::Fail,
                "qemu-system-arm not found (install qemu-system-arm)".into()
            )]
        );
        let modules = groups.iter().find(|g| g.group == Group::Modules).unwrap();
        assert_eq!(modules.level, Level::Warn);
        assert_eq!(modules.lines[0].1, "2/3 found, missing: nd_btt");
    }

    #[test]
    fn group_checks_healthy_report_is_one_line_per_group() {
        let groups = group_checks(&report(vec![
            check(
                Level::Pass,
                CheckKind::Config,
                "Configuration: virtuoso.toml found",
                Some("virtuoso.toml found"),
            ),
            check(
                Level::Pass,
                CheckKind::HostTools,
                "Host tools: all found (wget tar gcc)",
                Some("host tools"),
            ),
            check(Level::Pass, CheckKind::KernelPath, "KERNEL_PATH: /k", None),
            check(
                Level::Pass,
                CheckKind::KernelSource,
                "Kernel source: /k (v6.6.0)",
                Some("v6.6.0"),
            ),
            check(
                Level::Pass,
                CheckKind::KernelImage,
                "Kernel image: Image (42M)",
                Some("Image (42M)"),
            ),
            check(
                Level::Pass,
                CheckKind::QemuBinary,
                "QEMU binary: qemu-system-aarch64",
                Some("qemu-system-aarch64"),
            ),
            check(Level::Info, CheckKind::QemuImg, "qemu-img: available", None),
            check(
                Level::Info,
                CheckKind::Modules,
                "Kernel modules: none required",
                Some("none required"),
            ),
            check(
                Level::Pass,
                CheckKind::BusyBox,
                "BusyBox: cached (arm64)",
                Some("busybox"),
            ),
            check(
                Level::Info,
                CheckKind::ToolsImage,
                "Tools image: exists (attached as /dev/vdb)",
                Some("tools.img"),
            ),
            check(
                Level::Info,
                CheckKind::Initrd,
                "Initrd: /a/initrd.img (12M)",
                Some("initrd.img (12M)"),
            ),
        ]));
        let rendered = render(&groups, false);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 6, "六个组件组各一行:\n{rendered}");
        assert!(lines[0].contains("✓") && lines[0].contains("Config"));
        assert!(lines[2].contains("Kernel") && lines[2].contains("v6.6.0 · Image (42M)"));
        assert!(
            lines[5].contains("Artifacts")
                && lines[5].contains("busybox · tools.img · initrd.img (12M)")
        );
    }
}
