//! `virtuoso verify`：前置检查。builder 负责检查引擎，本模块负责
//! 把类型化配置投影为检查输入并呈现结果。

use common::fsutil::which;
use launcher::Arch;

use super::preset_kind;
use super::resolve_arch;
use crate::config::Config;

/// 检查引擎输入投影（verify 与 doctor 共用，保证检查语义单一来源——
/// 新增前置条件只动 builder::verify::run_checks，两侧呈现自动跟随）。
pub(super) fn engine_report(cfg: &Config, arch: Arch) -> anyhow::Result<builder::verify::Report> {
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
