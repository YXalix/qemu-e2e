//! VM 会话命令：shell / debug / test / matrix。
//! 启动 DSL 与双后端在 launcher；收割与看门狗在 guardian；判定在 judge。
//! 本模块做接线：构建（builder）→ 启动（launcher）→ 超时收割（guardian）→
//! 判定与工件（judge + runs）。

use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time::Instant;

use anyhow::Context;
use launcher::{Accel, Arch, Backend, QemuInvocation};

use super::{
    disk_opt, firecracker_kernel, firecracker_preflight, kernel_image_path, qemu_extra, resolve_arch,
    resolve_backend, resolve_topology,
};
use crate::config::Config;
use crate::runs;

pub fn run_shell(kvm: bool, backend: Option<&str>) -> anyhow::Result<i32> {
    run_vm_session(kvm, false, backend)
}

pub fn run_debug() -> anyhow::Result<i32> {
    println!("Starting QEMU with GDB stub on port 1234...");
    run_vm_session(false, true, None)
}

/// 交互式会话（shell / debug）：stdio 继承，不产运行工件。
/// firecracker 后端（仅交互 shell，gdb_stub 不适用）走 microVM 引导。
fn run_vm_session(kvm: bool, gdb_stub: bool, backend: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let arch = resolve_arch(&cfg, None)?;
    let topo = resolve_topology(&cfg)?;
    let backend = resolve_backend(&cfg, backend)?;
    if gdb_stub && backend == Backend::Firecracker {
        anyhow::bail!("gdb stub 仅 qemu 后端支持");
    }

    let status = if backend == Backend::Firecracker {
        let kernel = firecracker_kernel(&cfg, arch)?;
        firecracker_preflight(&cfg, arch, &kernel)?;
        println!("Starting firecracker microVM...");
        let inv = launcher::firecracker::FirecrackerInvocation::new(
            arch,
            &kernel,
            cfg.infra_dir.join("rootfs.img"),
            &topo,
            cfg.auto_test() == "1",
            cfg.infra_dir.join("firecracker-shell.json"),
            cfg.infra_dir.join("firecracker-shell.sock"),
        )
        .map_err(anyhow::Error::msg)?;
        let (mut child, mut sup) = inv.spawn_supervised(false)?;
        let st = child.wait().context("等待 firecracker 退出失败")?;
        sup.finish();
        st
    } else {
        let kernel = kernel_image_path(&cfg, arch)?;
        if kvm {
            println!("Starting QEMU with KVM acceleration...");
        } else if !gdb_stub {
            println!("Starting QEMU...");
        }

        let inv = QemuInvocation::new(
            arch,
            &kernel,
            cfg.infra_dir.join("initrd.img"),
            cfg.infra_dir.join("rootfs.img"),
        )
        .accel(if kvm { Accel::Kvm } else { Accel::Tcg })
        .topo(topo)
        .qemu_override(cfg.env.get("QEMU").as_deref())
        .disk(disk_opt(&cfg))
        .extra_opts(&qemu_extra(&cfg));

        let (mut child, mut sup) = inv.spawn_supervised(false)?;
        let st = child.wait().context("等待 QEMU 退出失败")?;
        sup.finish();
        st
    };
    Ok(super::code_of(status))
}

/// CI 模式：构建 → 启动（launcher）→ 超时看门狗（KILL 收割，124）→
/// judge 判定 → 运行工件。退出码 0=通过、124=超时、其余=失败。
/// `replay_until_fail > 1` 时返场重试：首个非 passed verdict 即停（tracker）。
pub fn run_test(
    cli_timeout: Option<u64>,
    cli_arch: Option<&str>,
    replay_until_fail: Option<u32>,
    cli_backend: Option<&str>,
) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let timeout_secs = resolve_timeout(&cfg, cli_timeout)?;
    let backend = resolve_backend(&cfg, cli_backend)?;
    let rounds = replay_until_fail.unwrap_or(1).max(1);
    let mut last_code = 1;
    for round in 1..=rounds {
        if rounds > 1 {
            println!("[REPLAY] round {round}/{rounds}");
        }
        last_code = test_once(&cfg, cli_arch, timeout_secs, backend)?;
        if last_code != 0 {
            if rounds > 1 {
                eprintln!("[REPLAY] aborted at round {round}/{rounds} (verdict not passed)");
            }
            return Ok(last_code);
        }
    }
    if rounds > 1 {
        println!("[REPLAY] {rounds}/{rounds} rounds passed — no flaky observed");
    }
    Ok(last_code)
}

fn resolve_timeout(cfg: &Config, cli_timeout: Option<u64>) -> anyhow::Result<u64> {
    let raw = cli_timeout
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.timeout_raw());
    if raw.trim() == "0" {
        anyhow::bail!("Set QEMU_TIMEOUT (e.g., cargo xtask test --timeout 60)");
    }
    raw.trim().parse().context("QEMU_TIMEOUT 必须是数字")
}

/// 单次完整测试（test 与 matrix 共用）。
fn test_once(cfg: &Config, cli_arch: Option<&str>, timeout_secs: u64, backend: Backend) -> anyhow::Result<i32> {
    let arch = resolve_arch(cfg, cli_arch)?;
    let topo = resolve_topology(cfg)?;
    let run = runs::create_run_dir(&cfg.project_root, arch.name())?;
    println!("[RUN] artifacts: {}", run.path.display());

    // ---- 构建（builder）----
    let build_started = Instant::now();
    if let Err(e) = super::build::build_pair_for(cfg, Some(&run.path.join("build.log"))) {
        let meta = build_failed_meta(&run, arch, timeout_secs, build_started.elapsed().as_millis() as u64);
        runs::finalize_run(&run, &meta)?;
        runs::prune(&cfg.project_root, runs::RUNS_KEEP);
        eprintln!("ERROR: initrd build failed; artifacts: {}", run.path.display());
        eprintln!("ERROR: {e:#}");
        return Ok(1);
    }

    // ---- 启动（launcher：qemu / firecracker 双后端，收割与判定同约定）----
    let started = Instant::now();
    let (mut child, mut sup, kernel, accel_label) = match backend {
        Backend::Firecracker => {
            let kernel = firecracker_kernel(cfg, arch)?;
            firecracker_preflight(cfg, arch, &kernel)?;
            let inv = launcher::firecracker::FirecrackerInvocation::new(
                arch,
                &kernel,
                cfg.infra_dir.join("rootfs.img"),
                &topo,
                cfg.auto_test() == "1",
                run.path.join("firecracker-config.json"),
                run.path.join("firecracker.sock"),
            )
            .map_err(anyhow::Error::msg)?;
            println!("Running firecracker test with {timeout_secs}s timeout...");
            let (child, sup) = inv.spawn_supervised(true)?;
            (child, sup, kernel, "KVM")
        }
        Backend::Qemu => {
            let kernel = kernel_image_path(cfg, arch)?;
            let inv = QemuInvocation::new(
                arch,
                &kernel,
                cfg.infra_dir.join("initrd.img"),
                cfg.infra_dir.join("rootfs.img"),
            )
            .accel(Accel::Tcg)
            .topo(topo.clone())
            .qemu_override(cfg.env.get("QEMU").as_deref())
            .disk(disk_opt(cfg))
            .auto_test(cfg.auto_test() == "1")
            .extra_opts(&qemu_extra(cfg));
            println!("Running QEMU test with {timeout_secs}s timeout...");
            let (child, sup) = inv.spawn_supervised(true)?;
            (child, sup, kernel, "TCG")
        }
    };
    // 墙钟看门狗：到点 KILL 进程组（等价 timeout --signal=KILL 的 124 语义）
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog = guardian::registry::spawn_watchdog(sup.pgid(), timeout_secs, Arc::clone(&timed_out));

    let status = runs::pump_child(
        &mut child,
        &run.path.join("serial.log"),
        &run.path.join("qemu-stderr.log"),
    )?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let timed_out_now = timed_out.load(Ordering::SeqCst);
    timed_out.store(true, Ordering::SeqCst);
    let _ = watchdog.join();

    // 退出码：超时 124；被信号杀死归一（137→124、信号死亡→130）；其余保留真实码。
    // 语义表单点在 judge::exit。
    let code = if timed_out_now {
        judge::exit::EXIT_TIMEOUT
    } else {
        let code = judge::exit::normalize(status.code());
        if status.code().is_none() {
            sup.kill_now();
        }
        code
    };
    sup.finish();

    // ---- 判定与工件（judge + runs）----
    let meta = runs::RunMeta {
        run_id: run.id.clone(),
        arch: arch.name().to_string(),
        exit_code: Some(code),
        duration_ms,
        timeout_s: timeout_secs.to_string(),
        kernel: Some(kernel),
        qemu_version: launcher::qemu_version(arch),
        topo: serde_json::json!({
            "smp": topo.smp.to_string(),
            "numa_nodes": topo.nodes.to_string(),
            "memory_per_node": topo.memory_per_node,
            "accel": accel_label,
            "backend": backend.name(),
            "auto_test": cfg.auto_test(),
        }),
        build_failed: false,
    };
    let verdict = runs::finalize_run(&run, &meta)?;
    runs::prune(&cfg.project_root, runs::RUNS_KEEP);

    match code {
        0 => println!("Test completed successfully!"),
        124 => eprintln!("ERROR: Test timed out after {timeout_secs} seconds"),
        _ => {}
    }
    println!(
        "[RUN] verdict: {verdict} (exit {code}, {:.1}s) — 详情: cargo xtask triage",
        duration_ms as f64 / 1000.0
    );
    Ok(code)
}

fn build_failed_meta(
    run: &runs::RunDir,
    arch: Arch,
    timeout_secs: u64,
    duration_ms: u64,
) -> runs::RunMeta {
    runs::RunMeta {
        run_id: run.id.clone(),
        arch: arch.name().to_string(),
        exit_code: Some(1),
        duration_ms,
        timeout_s: timeout_secs.to_string(),
        kernel: None,
        qemu_version: None,
        topo: serde_json::Value::Null,
        build_failed: true,
    }
}

/// 多架构矩阵：三架构（或 --arch 指定）串行执行完整测试，汇总总表。
/// 同宿主机串行避免资源争抢（CI 多 runner 天然并行）。
pub fn run_matrix(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let timeout_secs = resolve_timeout(&cfg, None)?;
    let arches: Vec<Arch> = match cli_arch.and_then(Arch::parse) {
        Some(a) => vec![a],
        None => launcher::ALL_ARCHES.to_vec(),
    };

    let mut results: Vec<(Arch, i32, String)> = Vec::new();
    for arch in arches {
        println!();
        println!("========== matrix: {} ==========", arch.name());
        let code = match test_once(&cfg, Some(arch.name()), timeout_secs, Backend::Qemu) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ERROR: {} — {e:#}", arch.name());
                1
            }
        };
        let verdict = runs::latest_run(&cfg.project_root)
            .and_then(|d| runs::load_verdict_or_parse(&d).ok())
            .map(|v| v.verdict.as_str().to_string())
            .unwrap_or_else(|| "unknown".into());
        results.push((arch, code, verdict));
    }

    println!();
    println!("========== MATRIX RESULTS ==========");
    println!("{:<8} {:<12} {:>5}", "ARCH", "VERDICT", "EXIT");
    let mut all_pass = true;
    for (arch, code, verdict) in &results {
        println!("{:<8} {:<12} {:>5}", arch.name(), verdict, code);
        if *code != 0 {
            all_pass = false;
        }
    }
    Ok(if all_pass { 0 } else { 1 })
}
