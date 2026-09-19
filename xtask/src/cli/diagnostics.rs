//! 配置诊断呈现（`cargo xtask verify` 的前置输出）。
//! 从 config 层拆出：config 只管解析与取值，本模块负责打印。

use launcher::{Arch, NumaTopology};

use crate::config::Config;

/// Phase 1 配置诊断。
pub fn print_diagnostics(cfg: &Config, arch_override: Option<&str>) {
    println!("[CONFIG] Virtuoso Phase-1 wrapper — typed config diagnostics");
    println!(
        "  .env: {}",
        cfg.env
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "absent (defaults; cp .env.example .env)".into())
    );
    if let Some(t) = &cfg.env.toml_path {
        println!("  virtuoso.toml: {} (overrides .env)", t.display());
    }

    // 架构
    let arch_raw = arch_override
        .map(str::to_string)
        .or_else(|| cfg.env.get("ARCH"));
    let host = Arch::host_default();
    match arch_raw.as_deref().and_then(Arch::parse).or(host) {
        Some(arch) => {
            let src = if arch_override.is_some() {
                "CLI --arch"
            } else if cfg.env.get("ARCH").is_some() {
                "config"
            } else {
                "host default"
            };
            println!(
                "  arch: {} ({src}; qemu={}, machine={}, console={})",
                arch.name(),
                arch.qemu_bin(),
                arch.machine(),
                arch.console()
            );
            if host.is_some_and(|h| h != arch) {
                println!(
                    "  WARN: cross-compile ARCH={} differs from host ({}), ensure cross-toolchain is available",
                    arch.name(),
                    std::env::consts::ARCH
                );
            }
        }
        None => {
            if let Some(raw) = arch_raw {
                println!("  ERROR: Unsupported ARCH={raw}");
            }
        }
    }

    // 内核路径与镜像
    match cfg.kernel_path() {
        Ok((kp, explicit)) => {
            println!(
                "  kernel_path: {} ({})",
                kp.display(),
                if explicit {
                    "from config"
                } else {
                    "auto-detected"
                }
            );
            if let Some(arch) = cfg.arch() {
                let img = kp.join(arch.kernel_img());
                println!(
                    "  kernel_image: {} — {}",
                    arch.kernel_img(),
                    if img.is_file() {
                        "found"
                    } else {
                        "MISSING (build the kernel first)"
                    }
                );
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    // CPU / NUMA
    let smp = cfg.env.get("SMP").unwrap_or_else(|| "8".into());
    let nodes_raw = cfg.env.get("NUMA_NODES").unwrap_or_else(|| "1".into());
    let mem = cfg.env.get("NUMA_MEMORY").unwrap_or_else(|| "1G".into());
    match NumaTopology::parse(&smp, &nodes_raw, &mem) {
        Ok(t) => {
            let total = t
                .total_memory()
                .unwrap_or_else(|_| format!("{mem} (unparsed)"));
            println!(
                "  cpu/mem: smp={}, numa_nodes={}, per-node={mem}, total={total}",
                t.smp, t.nodes
            );
        }
        Err(e) => println!("  WARN: {e}"),
    }

    // 超时
    let t = cfg.timeout_raw();
    println!(
        "  timeout: {t}s{}",
        if t == "0" {
            " (cargo xtask test will reject 0)"
        } else {
            ""
        }
    );

    // QEMU 二进制
    if let Some(arch) = cfg.arch() {
        let override_q = cfg.env.get("QEMU");
        let found = match &override_q {
            Some(q) => common::fsutil::which(q),
            None => common::fsutil::which(arch.qemu_bin()),
        };
        let label = override_q.as_deref().unwrap_or(arch.qemu_bin());
        println!(
            "  qemu: {label} — {}",
            if found {
                "found"
            } else {
                "NOT FOUND (install qemu-system or set QEMU=)"
            }
        );
    }
    println!();
}
