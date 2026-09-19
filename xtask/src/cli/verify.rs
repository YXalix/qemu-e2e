//! `cargo xtask verify`：前置检查。builder 负责检查引擎，本模块负责
//! 把类型化配置投影为检查输入并呈现结果；firecracker 后端追加 microVM
//! 诊断（launcher::firecracker::preflight_checks）。

use common::fsutil::which;
use launcher::{Arch, Backend};

use super::{firecracker_kernel, resolve_arch, resolve_backend};
use crate::config::Config;

pub fn run_verify(
    arch_override: Option<&str>,
    backend_override: Option<&str>,
) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    super::diagnostics::print_diagnostics(&cfg, arch_override);
    let backend = resolve_backend(&cfg, backend_override)?;
    println!("  backend: {}", backend.name());

    let arch = resolve_arch(&cfg, arch_override)?;
    let host_arch = Arch::parse(std::env::consts::ARCH);
    let kernel_path = cfg.kernel_path().ok().map(|(kp, _)| kp);
    let kernel_img = kernel_path.as_ref().map(|p| p.join(arch.kernel_img()));

    // modules.conf 声明的模块是否都能在内核树找到（WARN 级；输入收集在 builder）
    let modules_conf = cfg.infra_dir.join("modules.conf");
    let modules =
        builder::verify::module_presence(&modules_conf, kernel_path.as_deref(), &cfg.infra_dir);

    let report = builder::verify::run_checks(
        cfg.env.path.is_some(),
        kernel_path.as_deref(),
        arch,
        host_arch.is_some_and(|h| h != arch),
        kernel_img.as_deref(),
        which(arch.qemu_bin()).then_some(arch.qemu_bin()),
        cfg.env.get("QEMU").as_deref(),
        &modules,
        modules_conf.is_file(),
        cfg.infra_dir
            .join("busybox/bin")
            .join(format!("busybox-{}", arch.name()))
            .is_file(),
        cfg.infra_dir.join("disk.qcow2").is_file(),
        Some(&cfg.infra_dir.join("initrd.img")),
    );
    print!("{}", report.render());

    // firecracker 后端追加 microVM preflight（只诊断不改判 qemu 侧结论）
    if backend == Backend::Firecracker {
        println!("  firecracker preflight:");
        let kernel = firecracker_kernel(&cfg, arch)?;
        let kernel_config = cfg.kernel_path().ok().map(|(kp, _)| kp.join(".config"));
        let checks = launcher::firecracker::preflight_checks(
            arch,
            &kernel,
            kernel_config.as_deref(),
            cfg.env
                .get("FIRECRACKER_BIN")
                .as_deref()
                .unwrap_or("firecracker"),
        );
        let mut fc_fail = 0;
        for chk in &checks {
            if !chk.ok {
                fc_fail += 1;
            }
            println!(
                "    [{}] {} — {}",
                if chk.ok { "OK" } else { "FAIL" },
                chk.name,
                chk.note
            );
        }
        if fc_fail > 0 {
            println!();
            println!("  firecracker preflight 未通过（{fc_fail} 项）——qemu 后端不受影响");
            return Ok(1);
        }
    }

    if report.critical_fail > 0 {
        println!();
        println!("  Fix the issues above, then run: cargo xtask verify");
        Ok(1)
    } else {
        println!();
        println!("  Ready. Run: cargo xtask test --timeout 30");
        Ok(0)
    }
}
