//! `virtuoso verify`：前置检查。builder 负责检查引擎，本模块负责
//! 把类型化配置投影为检查输入并呈现结果。

use common::fsutil::which;
use launcher::Arch;

use super::resolve_arch;
use crate::config::Config;

/// 检查引擎输入投影（verify 与 doctor 共用，保证检查语义单一来源——
/// 新增前置条件只动 builder::verify::run_checks，两侧呈现自动跟随）。
pub(super) fn engine_report(cfg: &Config, arch: Arch) -> anyhow::Result<builder::verify::Report> {
    let host_arch = Arch::parse(std::env::consts::ARCH);
    let kernel_path = cfg.kernel_path().ok().map(|(kp, _)| kp);
    let kernel_img = kernel_path.as_ref().map(|p| p.join(arch.kernel_img()));

    // 组件 require 并集（boot 附加 + runtime）的模块是否都能在内核树找到
    // （WARN 级；并集计算在 config 层）
    let plan = cfg.component_plan();
    let module_lines: Vec<String> = plan.all().cloned().collect();
    let modules =
        builder::verify::module_presence(&module_lines, kernel_path.as_deref(), &cfg.infra_dir);

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
    ))
}

pub fn run_verify(arch_override: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    super::diagnostics::print_diagnostics(&cfg, arch_override);

    let arch = resolve_arch(&cfg, arch_override)?;
    let report = engine_report(&cfg, arch)?;
    print!("{}", report.render());

    if report.critical_fail > 0 {
        println!();
        println!("  Fix the issues above, then run: virtuoso verify");
        Ok(1)
    } else {
        println!();
        println!("  Ready. Run: virtuoso test --timeout 30");
        Ok(0)
    }
}
