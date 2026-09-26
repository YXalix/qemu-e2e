//! VM 会话命令：shell / test / matrix。
//! 启动 DSL 在 launcher；收割与看门狗在 guardian；判定在 judge。
//! 本模块做接线：构建（builder）→ 启动（launcher）→ 超时收割（guardian）→
//! 判定与工件（judge + runs）。

use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time::Instant;

use anyhow::Context;
use launcher::{Accel, Arch, HostOs};

use super::{build_invocation, open_agent_socket, resolve_arch, resolve_topology, LaunchPlan};
use crate::config::Config;
use crate::runs;

/// CLI accel 解析：`--kvm` / `--tcg` 显式指定，缺省走平台规则
/// （macOS 同构 HVF，其余 TCG）。`--kvm --tcg` 互斥。
pub(crate) fn resolve_accel(kvm: bool, tcg: bool, arch: Arch) -> anyhow::Result<Accel> {
    if kvm && tcg {
        anyhow::bail!("--kvm 与 --tcg 互斥");
    }
    Ok(if kvm {
        Accel::Kvm
    } else if tcg {
        Accel::Tcg
    } else {
        Accel::default_for(arch, HostOs::current(), Arch::host_default())
    })
}

pub fn run_shell(kvm: bool, tcg: bool, gdb: bool) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let arch = resolve_arch(&cfg, None)?;
    if gdb && kvm {
        anyhow::bail!("--gdb 与 --kvm 互斥（GDB 单步调试走 TCG 纯模拟）");
    }
    // --gdb：挂起等 GDB 连接，恒 TCG（单步可靠）；其余走平台 accel 规则
    let accel = if gdb {
        Accel::Tcg
    } else {
        resolve_accel(kvm, tcg, arch)?
    };
    run_vm_session(accel, gdb)
}

/// 交互式会话（shell，含 --gdb 调试挂起）：stdio 继承，不产运行工件。
fn run_vm_session(accel: Accel, gdb_stub: bool) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let arch = resolve_arch(&cfg, None)?;
    let topo = resolve_topology(&cfg)?;
    if gdb_stub {
        println!("Starting QEMU with GDB stub on port 1234...");
    }
    match accel {
        Accel::Kvm => println!("Starting QEMU with KVM acceleration..."),
        Accel::Hvf => println!("Starting QEMU with HVF acceleration..."),
        Accel::Tcg => {
            if !gdb_stub {
                println!("Starting QEMU...");
            }
        }
    }

    let inv = build_invocation(&LaunchPlan {
        cfg: &cfg,
        arch,
        topo: topo.clone(),
        accel,
        auto_test: false,
        agent_socket: open_agent_socket(&cfg, &cfg.target_dir, "agent-shell"),
    })?;

    println!("[LAUNCH] {}", inv.command_line()?);
    let (mut child, mut sup) = inv.spawn_supervised(false)?;
    let st = child.wait().context("等待 QEMU 退出失败")?;
    sup.finish();
    Ok(super::code_of(st))
}

/// CI 模式：构建 → 启动（launcher）→ 超时看门狗（KILL 收割，124）→
/// judge 判定 → 运行工件。退出码 0=通过、124=超时、其余=失败。
/// `replay_until_fail > 1` 时返场重试：首个非 passed verdict 即停（tracker）。
/// accel 缺省走平台规则（macOS 同构 HVF，Linux 恒 TCG）；`--tcg` 强制纯模拟。
pub fn run_test(
    cli_timeout: Option<u64>,
    cli_arch: Option<&str>,
    replay_until_fail: Option<u32>,
    tcg: bool,
) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let timeout_secs = resolve_timeout(&cfg, cli_timeout)?;
    let rounds = replay_until_fail.unwrap_or(1).max(1);
    let mut last_code = 1;
    for round in 1..=rounds {
        if rounds > 1 {
            println!("[REPLAY] round {round}/{rounds}");
        }
        last_code = test_once(&cfg, cli_arch, timeout_secs, tcg)?;
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
        anyhow::bail!("Set QEMU_TIMEOUT (e.g., virtuoso test --timeout 60)");
    }
    raw.trim().parse().context("QEMU_TIMEOUT 必须是数字")
}

/// 单次完整测试（test 与 matrix 共用）。
fn test_once(
    cfg: &Config,
    cli_arch: Option<&str>,
    timeout_secs: u64,
    tcg: bool,
) -> anyhow::Result<i32> {
    let arch = resolve_arch(cfg, cli_arch)?;
    let accel = resolve_accel(false, tcg, arch)?;
    let topo = resolve_topology(cfg)?;
    let run = runs::create_run_dir(&cfg.project_root, arch.name())?;
    println!("[RUN] artifacts: {}", run.path.display());

    // ---- 构建（builder）----
    let build_started = Instant::now();
    if let Err(e) = super::build::build_pair_for(cfg, Some(&run.path.join("build.log")), &[]) {
        let meta = build_failed_meta(
            &run,
            arch,
            timeout_secs,
            build_started.elapsed().as_millis() as u64,
        );
        runs::finalize_run(&run, &meta)?;
        runs::prune(&cfg.project_root, runs::RUNS_KEEP);
        // 单一 ERROR 打印点在 main（bail 携带现场与工件路径）
        return Err(e.context(format!(
            "initrd build failed; artifacts: {}",
            run.path.display()
        )));
    }

    // ---- 启动（launcher；收割与判定在 guardian/judge）----
    let started = Instant::now();
    let inv = build_invocation(&LaunchPlan {
        cfg,
        arch,
        topo: topo.clone(),
        accel,
        auto_test: cfg.auto_test(),
        agent_socket: open_agent_socket(cfg, &run.path, "agent"),
    })?;
    println!("[LAUNCH] {}", inv.command_line()?);
    println!("Running QEMU test with {timeout_secs}s timeout...");
    let (mut child, mut sup) = inv.spawn_supervised(true)?;

    // 墙钟看门狗：到点 KILL 进程组（等价 timeout --signal=KILL 的 124 语义）
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog =
        guardian::registry::spawn_watchdog(sup.pgid(), timeout_secs, Arc::clone(&timed_out));

    let pumped = runs::pump_child(
        &mut child,
        &run.path.join("serial.log"),
        &run.path.join("qemu-stderr.log"),
    )?;
    let status = pumped.status;
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

    // QEMU 自身输出与 guest 串口分离呈现：失败/超时时补看 stderr 尾部
    // （QEMU 早夭或参数被拒的现场）；成功保持安静。
    pumped.report_tail_on_failure(code);

    // ---- 判定与工件（judge + runs）----
    let meta = runs::RunMeta {
        run_id: run.id.clone(),
        arch: arch.name().to_string(),
        exit_code: Some(code),
        duration_ms,
        timeout_s: timeout_secs.to_string(),
        kernel: Some(super::kernel_image_path(cfg, arch)?),
        qemu_version: launcher::qemu_version(arch),
        topo: serde_json::json!({
            "smp": topo.smp.to_string(),
            "numa_nodes": topo.nodes.to_string(),
            "memory_per_node": topo.memory_per_node,
            "accel": accel.label(),
            "auto_test": cfg.auto_test().to_string(),
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
        "[RUN] verdict: {verdict} (exit {code}, {:.1}s) — 详情: virtuoso triage",
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
        let code = match test_once(&cfg, Some(arch.name()), timeout_secs, false) {
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
