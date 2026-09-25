//! `virtuoso doctor`：环境体检唯一入口。检查引擎复用
//! builder::verify::run_checks（语义单一来源，引擎输入投影 engine_report
//! 在本文件），两种呈现：缺省 = flutter-doctor 风格一屏（引擎检查按消息
//! 前缀归并为组件行，✓/✗/! 一眼可读）；`--verbose` = 类型化配置诊断 +
//! 完整检查清单。退出码：critical 未过 → 1。

use common::fsutil::which;
use launcher::Arch;

use builder::verify::{Level, Report};

use super::{preset_kind, resolve_arch};
use crate::config::Config;

// ---------------------------------------------------------------- 引擎输入投影

/// 检查引擎输入投影（保证检查语义单一来源——新增前置条件只动
/// builder::verify::run_checks，doctor 的两种呈现自动跟随）。
fn engine_report(cfg: &Config, arch: Arch) -> anyhow::Result<Report> {
    let host_arch = Arch::parse(std::env::consts::ARCH);
    let kernel_path = cfg.kernel_path().ok().map(|(kp, _)| kp);
    let preset = preset_kind(cfg)?;
    let preset_active = preset.is_some();

    // preset 激活：内核镜像 = fetch 缓存（离线呈现钉定版本，未钉定留空）；
    // 源码树 .ko 查找整体跳过（预编内核全 =y 内建，无怪癖前置）
    let kernel_img = if preset_active {
        Some(super::preset_dir(cfg).join(builder::preset::image_name(arch)))
    } else {
        kernel_path.as_ref().map(|p| p.join(arch.kernel_img()))
    };
    let modules = if preset_active {
        Vec::new()
    } else {
        let plan = cfg.component_plan();
        let module_lines: Vec<String> = plan.all().cloned().collect();
        builder::verify::module_presence(&module_lines, kernel_path.as_deref(), &cfg.infra_dir)
    };
    let preset_version = if preset_active {
        builder::preset::pin_version(&cfg.infra_dir).unwrap_or_default()
    } else {
        String::new()
    };

    Ok(builder::verify::run_checks(
        cfg.toml.is_some(),
        kernel_path.as_deref(),
        arch,
        common::HostOs::current(),
        host_arch.is_some_and(|h| h != arch),
        kernel_img.as_deref(),
        which(arch.qemu_bin()).then_some(arch.qemu_bin()),
        cfg.qemu_override().as_deref(),
        &modules,
        cfg.build_dir
            .join("busybox/bin")
            .join(format!("busybox-{}", arch.name()))
            .is_file(),
        cfg.artifacts_dir.join("tools.img").is_file(),
        Some(&cfg.artifacts_dir.join("initrd.img")),
        cfg.vfio().is_some(),
        cfg.pmem_size().is_some(),
        preset.as_deref().map(|_| preset_version.as_str()),
    ))
}

/// docker 供给模式附加检查（forge 活动卷）：状态文件存在 = 活动卷开启，
/// 才投影 forge 检查——raw/preset 用户无状态文件，零打扰。
fn docker_report(cfg: &Config, report: &mut Report) {
    let Ok(Some(current)) = forge::state::read(&cfg.project_root) else {
        return;
    };
    let volume = std::env::var("KERNEL_VOLUME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(current.volume);
    let engine_err = forge::volume::engine_guard().err().map(|e| e.to_string());
    let host_view = forge::volume::host_view(&volume).ok();
    let image = std::env::var(forge::toolchain::IMAGE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| forge::DEFAULT_IMAGE.to_string());
    let image_present = forge::toolchain::image_present(&image);
    report.extend(builder::verify::kernel_docker_checks(
        &volume,
        &current.arch,
        engine_err.as_deref(),
        host_view.as_deref(),
        image_present,
        &image,
    ));
}

// ---------------------------------------------------------------- 引擎消息 → 组件分组

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Config,
    Toolchain,
    Kernel,
    Qemu,
    Modules,
    Artifacts,
    Other,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Config => "Config",
            Group::Toolchain => "Toolchain",
            Group::Kernel => "Kernel",
            Group::Qemu => "QEMU",
            Group::Modules => "Modules",
            Group::Artifacts => "Artifacts",
            Group::Other => "Other",
        }
    }
}

/// 固定呈现顺序（Other 恒最后，无内容则不渲染）。
const GROUP_ORDER: &[Group] = &[
    Group::Config,
    Group::Toolchain,
    Group::Kernel,
    Group::Qemu,
    Group::Modules,
    Group::Artifacts,
    Group::Other,
];

/// 引擎检查消息的已知前缀 → 分组。
const PREFIXES: &[(&str, Group)] = &[
    ("Configuration", Group::Config),
    ("Host tools", Group::Toolchain),
    ("Cross-compile", Group::Toolchain),
    ("Kernel modules", Group::Modules),
    ("Kernel preset", Group::Kernel),
    ("Kernel docker", Group::Kernel),
    ("Kernel source", Group::Kernel),
    ("Kernel image", Group::Kernel),
    ("KERNEL_PATH", Group::Kernel),
    ("QEMU binary", Group::Qemu),
    ("qemu-img", Group::Qemu),
    ("BusyBox", Group::Artifacts),
    ("Tools image", Group::Artifacts),
    ("Initrd", Group::Artifacts),
    ("Components", Group::Modules),
];

/// 消息 → (分组, 剥掉前缀后的正文)；未识别前缀归 Other 原样保留
/// （引擎未来新增检查不至于在 doctor 里丢行）。
fn classify(msg: &str) -> (Group, &str) {
    for (prefix, group) in PREFIXES {
        if let Some(rest) = msg.strip_prefix(prefix) {
            return (*group, rest.trim_start_matches([':', ' ']));
        }
    }
    (Group::Other, msg)
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

/// Pass/Info 检查的组内紧凑短语；Fail/Warn 不走这里（原样保留修复提示）。
fn short_detail(msg: &str) -> String {
    let (_, rest) = classify(msg);
    // 可选项（qemu-img）与 Kernel source 行重复的裸路径不进体检行
    if msg.starts_with("qemu-img") || msg.starts_with("KERNEL_PATH") {
        return String::new();
    }
    // "Kernel source: /p (v6.6.0)" → "v6.6.0"
    if msg.starts_with("Kernel source") {
        if let Some((_, tail)) = rest.split_once(" (v") {
            return format!("v{}", tail.trim_end_matches(')'));
        }
    }
    // "Kernel preset: mainline v6.12.8 (…)" → "mainline v6.12.8"
    if msg.starts_with("Kernel preset") {
        return rest
            .split_once(" (")
            .map(|(head, _)| head.to_string())
            .unwrap_or_else(|| rest.to_string());
    }
    if msg.starts_with("Kernel modules") && rest.starts_with("preset") {
        return "modules built-in".into();
    }
    // docker 供给组：engine 行无信息量跳过；volume/image 压成短语
    if msg.starts_with("Kernel docker: engine") {
        return String::new();
    }
    if msg.starts_with("Kernel docker: volume") {
        let v = rest
            .split_whitespace()
            .nth(1) // rest 以 "volume <名> …" 开头，取卷名
            .unwrap_or("");
        return format!("docker volume {v}");
    }
    if msg.starts_with("Kernel docker: toolchain image") {
        return if rest.contains("not pulled") {
            "toolchain image (not pulled)".into()
        } else {
            "toolchain image".into()
        };
    }
    if msg.starts_with("Host tools") {
        return "host tools".into();
    }
    if msg.starts_with("BusyBox") {
        return if rest.starts_with("cached") {
            "busybox".into()
        } else {
            "busybox (not cached)".into()
        };
    }
    if msg.starts_with("Tools image") {
        return if rest.starts_with("exists") {
            "tools.img".into()
        } else {
            "tools.img (not built)".into()
        };
    }
    if msg.starts_with("Initrd") {
        return if rest.starts_with("not built") {
            "initrd (not built)".into()
        } else {
            basenameify(rest)
        };
    }
    rest.to_string()
}

/// "/p/target/artifacts/initrd.img (12M)" → "initrd.img (12M)"。
fn basenameify(rest: &str) -> String {
    fn name(p: &str) -> &str {
        std::path::Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(p)
    }
    match rest.split_once(' ') {
        Some((p, tail)) => format!("{} {tail}", name(p)),
        None => name(rest).to_string(),
    }
}

// ---------------------------------------------------------------- 组装与呈现

#[derive(Debug, PartialEq)]
struct GroupOut {
    group: Group,
    level: Level,
    /// (级别, 文本)：Pass/Info 为紧凑短语（合并渲染），Fail/Warn 为全文（逐行）。
    lines: Vec<(Level, String)>,
}

fn slot(out: &mut [GroupOut], g: Group) -> &mut GroupOut {
    out.iter_mut()
        .find(|s| s.group == g)
        .expect("GROUP_ORDER 覆盖全部分组")
}

fn group_checks(report: &Report) -> Vec<GroupOut> {
    let mut out: Vec<GroupOut> = GROUP_ORDER
        .iter()
        .map(|g| GroupOut {
            group: *g,
            level: Level::Info,
            lines: Vec::new(),
        })
        .collect();
    for chk in &report.checks {
        let (g, _) = classify(&chk.msg);
        let s = slot(&mut out, g);
        s.level = worst(s.level, chk.level);
        match chk.level {
            Level::Pass | Level::Info => {
                let short = short_detail(&chk.msg);
                if !short.is_empty() && !s.lines.iter().any(|(_, l)| *l == short) {
                    s.lines.push((chk.level, short));
                }
            }
            Level::Fail | Level::Warn => {
                s.lines.push((chk.level, classify(&chk.msg).1.to_string()));
            }
        }
    }
    out.retain(|s| !s.lines.is_empty());
    out
}

fn render(groups: &[GroupOut], tty: bool) -> String {
    let c = |code: &str, s: &str| {
        if tty {
            format!("\x1b[{code}m{s}\x1b[0m")
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

// ---------------------------------------------------------------- 入口

pub fn run_doctor(arch_override: Option<&str>, json: bool, verbose: bool) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let arch = resolve_arch(&cfg, arch_override)?;
    let mut report = engine_report(&cfg, arch)?;
    docker_report(&cfg, &mut report);

    // --verbose：全量呈现（类型化配置诊断 + 完整检查清单），文本态专属
    if verbose && !json {
        super::diagnostics::print_diagnostics(&cfg, arch_override);
        print!("{}", report.render());
        println!();
        if report.critical_fail > 0 {
            println!("  Fix the issues above, then re-run: virtuoso doctor --verbose");
        } else {
            println!("  Ready. Run: virtuoso test --timeout 30");
        }
        return Ok(i32::from(report.critical_fail > 0));
    }

    let groups = group_checks(&report);

    let fail_total = report.critical_fail;
    if json {
        let level = |l: Level| match l {
            Level::Fail => "fail",
            Level::Warn => "warn",
            _ => "pass",
        };
        let out = serde_json::json!({
            "arch": arch.name(),
            "ok": fail_total == 0,
            "critical_fail": fail_total,
            "warnings": report.warnings,
            "groups": groups
                .iter()
                .map(|g| serde_json::json!({
                    "name": g.group.name(),
                    "level": level(g.level),
                    "details": g.lines.iter().map(|(_, t)| t).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        let tty = builder::verify::is_stdout_tty();
        println!("Virtuoso doctor · arch {}", arch.name());
        print!("{}", render(&groups, tty));
        if fail_total > 0 {
            println!(
                "\n  {} {fail_total} critical, {} warnings — full checklist: virtuoso doctor --verbose",
                (if tty { "\x1b[0;31m✗\x1b[0m" } else { "✗" }),
                report.warnings
            );
        } else {
            println!(
                "\n  {} Ready — virtuoso test --timeout {}",
                (if tty { "\x1b[0;32m✓\x1b[0m" } else { "✓" }),
                cfg.timeout_raw()
            );
        }
    }
    Ok(i32::from(fail_total > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use builder::verify::Check;

    fn check(level: Level, msg: &str) -> Check {
        Check {
            level,
            msg: msg.into(),
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
    fn classify_maps_every_engine_prefix() {
        for (msg, want, rest) in [
            (
                "Configuration: virtuoso.toml found",
                Group::Config,
                "virtuoso.toml found",
            ),
            (
                "Host tools: all found (wget …)",
                Group::Toolchain,
                "all found (wget …)",
            ),
            (
                "Cross-compile: ARCH=arm64 differs",
                Group::Toolchain,
                "ARCH=arm64 differs",
            ),
            ("KERNEL_PATH: /k", Group::Kernel, "/k"),
            ("Kernel source: /k (v6.6)", Group::Kernel, "/k (v6.6)"),
            ("Kernel image: Image (42M)", Group::Kernel, "Image (42M)"),
            (
                "Kernel docker: engine ok",
                Group::Kernel,
                "engine ok",
            ),
            (
                "Kernel docker: volume ksrc-oe66 reachable (/v) [arch arm64]",
                Group::Kernel,
                "volume ksrc-oe66 reachable (/v) [arch arm64]",
            ),
            (
                "QEMU binary: qemu-system-aarch64",
                Group::Qemu,
                "qemu-system-aarch64",
            ),
            ("qemu-img: available", Group::Qemu, "available"),
            (
                "Kernel modules: 1/2 found, missing: x",
                Group::Modules,
                "1/2 found, missing: x",
            ),
            (
                "BusyBox: cached (arm64)",
                Group::Artifacts,
                "cached (arm64)",
            ),
            ("Tools image: exists", Group::Artifacts, "exists"),
            (
                "Initrd: /a/initrd.img (12M)",
                Group::Artifacts,
                "/a/initrd.img (12M)",
            ),
            ("Future check: ???", Group::Other, "Future check: ???"),
        ] {
            assert_eq!(classify(msg), (want, rest), "msg={msg}");
        }
    }

    #[test]
    fn worst_orders_fail_over_warn_over_pass() {
        assert_eq!(worst(Level::Info, Level::Pass), Level::Pass);
        assert_eq!(worst(Level::Pass, Level::Warn), Level::Warn);
        assert_eq!(worst(Level::Warn, Level::Fail), Level::Fail);
    }

    #[test]
    fn short_detail_compacts_known_passes() {
        assert_eq!(
            short_detail("Configuration: virtuoso.toml found"),
            "virtuoso.toml found"
        );
        assert_eq!(
            short_detail("Host tools: all found (wget tar gcc)"),
            "host tools"
        );
        assert_eq!(short_detail("Kernel source: /k (v6.6.0)"), "v6.6.0");
        assert_eq!(short_detail("Kernel image: Image (42M)"), "Image (42M)");
        assert_eq!(short_detail("qemu-img: available"), "");
        assert_eq!(short_detail("KERNEL_PATH: /k"), "");
        assert_eq!(short_detail("Kernel docker: engine ok"), "");
        assert_eq!(
            short_detail("Kernel docker: volume ksrc-oe66 reachable (/v) [arch arm64]"),
            "docker volume ksrc-oe66"
        );
        assert_eq!(
            short_detail("Kernel docker: toolchain image ghcr.io/x/virtuoso-kernel:latest"),
            "toolchain image"
        );
        assert_eq!(
            short_detail("Kernel docker: toolchain image not pulled yet (auto-pull …)"),
            "toolchain image (not pulled)"
        );
        assert_eq!(
            short_detail("Kernel modules: none required"),
            "none required"
        );
        assert_eq!(short_detail("BusyBox: cached (arm64)"), "busybox");
        assert_eq!(
            short_detail("BusyBox: not cached for arm64 (…)"),
            "busybox (not cached)"
        );
        assert_eq!(
            short_detail("Tools image: exists (attached as /dev/vdb)"),
            "tools.img"
        );
        assert_eq!(
            short_detail("Tools image: not built yet (run `virtuoso build`)"),
            "tools.img (not built)"
        );
        assert_eq!(
            short_detail("Initrd: /a/b/initrd.img (12M)"),
            "initrd.img (12M)"
        );
        assert_eq!(
            short_detail("Initrd: not built yet (run `virtuoso build`)"),
            "initrd (not built)"
        );
    }

    #[test]
    fn group_checks_fails_lead_and_warn_keeps_full_text() {
        let groups = group_checks(&report(vec![
            check(Level::Pass, "Configuration: virtuoso.toml found"),
            check(
                Level::Fail,
                "QEMU binary: qemu-system-arm not found (install qemu-system-arm)",
            ),
            check(Level::Warn, "Kernel modules: 2/3 found, missing: nd_btt"),
            check(Level::Pass, "Kernel image: Image (42M)"),
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
            check(Level::Pass, "Configuration: virtuoso.toml found"),
            check(Level::Pass, "Host tools: all found (wget tar gcc)"),
            check(Level::Pass, "KERNEL_PATH: /k"),
            check(Level::Pass, "Kernel source: /k (v6.6.0)"),
            check(Level::Pass, "Kernel image: Image (42M)"),
            check(Level::Pass, "QEMU binary: qemu-system-aarch64"),
            check(Level::Info, "qemu-img: available"),
            check(Level::Info, "Kernel modules: none required"),
            check(Level::Pass, "BusyBox: cached (arm64)"),
            check(Level::Info, "Tools image: exists (attached as /dev/vdb)"),
            check(Level::Info, "Initrd: /a/initrd.img (12M)"),
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
