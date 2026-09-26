//! `virtuoso doctor`：环境体检唯一入口。检查引擎复用
//! crate::builder::verify::run_checks（语义单一来源，引擎输入投影 engine_report
//! 在本文件），两种呈现：缺省 = flutter-doctor 风格一屏（引擎检查按消息
//! 前缀归并为组件行，✓/✗/! 一眼可读，实现在 `screen`）；`--verbose` =
//! 类型化配置诊断 + 完整检查清单。退出码：critical 未过 → 1。

use crate::fsutil::which;
use crate::Arch;

use crate::builder::verify::{Level, Report};

use super::resolve_arch;
use crate::config::Config;

mod screen;

// ---------------------------------------------------------------- 引擎输入投影

/// 检查引擎输入投影（保证检查语义单一来源——新增前置条件只动
/// crate::builder::verify::run_checks，doctor 的两种呈现自动跟随）。
fn engine_report(cfg: &Config, arch: Arch) -> anyhow::Result<Report> {
    let host_arch = Arch::parse(std::env::consts::ARCH);
    let kernel_path = cfg.kernel_path().ok().map(|(kp, _)| kp);

    let kernel_img = kernel_path.as_ref().map(|p| p.join(arch.kernel_img()));
    let plan = cfg.component_plan();
    let module_lines: Vec<String> = plan.all().cloned().collect();
    let modules = crate::builder::verify::module_presence(&module_lines, kernel_path.as_deref(), &cfg.infra_dir);

    Ok(crate::builder::verify::run_checks(&crate::builder::verify::CheckInput {
        config_file_exists: cfg.toml.is_some(),
        kernel_path: kernel_path.as_deref(),
        arch,
        host: crate::HostOs::current(),
        host_is_cross: host_arch.is_some_and(|h| h != arch),
        kernel_image: kernel_img.as_deref(),
        qemu_bin: which(arch.qemu_bin()).then_some(arch.qemu_bin()),
        qemu_override: cfg.qemu_override().as_deref(),
        modules: &modules,
        busybox_cached: cfg
            .build_dir
            .join("busybox/bin")
            .join(format!("busybox-{}", arch.name()))
            .is_file(),
        tools_img_exists: cfg.artifacts_dir.join("tools.img").is_file(),
        initrd: Some(&cfg.artifacts_dir.join("initrd.img")),
        vfio_enabled: cfg.vfio().is_some(),
        pmem_enabled: cfg.pmem_size().is_some(),
    }))
}

/// docker 供给模式附加检查（forge 活动卷）：状态文件存在 = 活动卷开启，
/// 才投影 forge 检查——raw 用户无状态文件，零打扰。
fn docker_report(cfg: &Config, report: &mut Report) {
    let Ok(Some(current)) = crate::forge::state::read(&cfg.project_root) else {
        return;
    };
    let volume = std::env::var("KERNEL_VOLUME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(current.volume);
    let engine_err = crate::forge::volume::engine_guard().err().map(|e| e.to_string());
    let host_view = crate::forge::volume::host_view(&volume).ok();
    let image = std::env::var(crate::forge::toolchain::IMAGE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::forge::DEFAULT_IMAGE.to_string());
    let image_present = crate::forge::toolchain::image_present(&image);
    report.extend(crate::builder::verify::kernel_docker_checks(
        &volume,
        &current.arch,
        engine_err.as_deref(),
        host_view.as_deref(),
        image_present,
        &image,
    ));
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

    let groups = screen::group_checks(&report);

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
        let tty = crate::builder::verify::is_stdout_tty();
        let (pass_icon, fail_icon) = (
            crate::ui::icon(crate::ui::Icon::Pass),
            crate::ui::icon(crate::ui::Icon::Fail),
        );
        println!("Virtuoso doctor · arch {}", arch.name());
        print!("{}", screen::render(&groups, tty));
        if fail_total > 0 {
            println!(
                "\n  {} {fail_total} critical, {} warnings — full checklist: virtuoso doctor --verbose",
                fail_icon,
                report.warnings
            );
        } else {
            let suggest = match cfg.timeout_raw() {
                t if t.trim() == "0" => "virtuoso test --timeout 60".to_string(),
                t => format!("virtuoso test --timeout {}", t.trim()),
            };
            println!("\n  {pass_icon} Ready — {suggest}");
        }
    }
    Ok(i32::from(fail_total > 0))
}
