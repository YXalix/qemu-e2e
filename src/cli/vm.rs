//! VM 会话命令：shell / test。
//! 启动 DSL、进程治理与看门狗在 launcher；判定在 judge。
//! 本模块做接线：构建（builder）→ 启动（launcher）→ 超时收割 →
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
        anyhow::bail!("--kvm and --tcg are mutually exclusive");
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
        anyhow::bail!("--gdb and --kvm are mutually exclusive (single-stepping needs TCG)");
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
        only_tests: Vec::new(),
        agent_socket: open_agent_socket(&cfg, &cfg.target_dir, "agent-shell"),
    })?;

    println!("[LAUNCH] {}", inv.command_line()?);
    let (mut child, mut sup) = inv.spawn_supervised(false)?;
    let st = child.wait().context("failed to wait for QEMU exit")?;
    sup.finish();
    Ok(super::code_of(st))
}

/// CI 模式：构建 → 启动（launcher）→ 超时看门狗（KILL 收割，124）→
/// judge 判定 → 运行工件。退出码 0=通过、124=超时、其余=失败。
/// `replay_until_fail > 1` 时返场重试：首个非 passed verdict 即停。
/// accel 缺省走平台规则（macOS 同构 HVF，Linux 恒 TCG）；`--tcg` 强制纯模拟。
/// `only` 非空时 cmdline 追加 virtuoso.only=，init 只跑名单内的 /tests 二进制。
pub fn run_test(
    cli_timeout: Option<u64>,
    cli_arch: Option<&str>,
    replay_until_fail: Option<u32>,
    tcg: bool,
    only: &[String],
) -> anyhow::Result<i32> {
    validate_only(only)?;
    let cfg = Config::load()?;
    let timeout_secs = resolve_timeout(&cfg, cli_timeout)?;
    let rounds = replay_until_fail.unwrap_or(1).max(1);
    let mut last_code = 1;
    for round in 1..=rounds {
        if rounds > 1 {
            println!("[REPLAY] round {round}/{rounds}");
        }
        let accel = if tcg { Some(Accel::Tcg) } else { None };
        let (code, _) = test_once(&cfg, cli_arch, timeout_secs, accel, only)?;
        last_code = code;
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

/// `--only` 名单合法性：测例名是文件 basename——非空、无逗号/空白
/// （逗号是 init 侧的名单分隔符，空白会拆坏 cmdline token）。
fn validate_only(only: &[String]) -> anyhow::Result<()> {
    for name in only {
        if name.trim().is_empty() || name.contains(|c: char| c.is_whitespace() || c == ',') {
            anyhow::bail!(
                "invalid --only entry {name:?}: test names are file basenames (no commas/whitespace)"
            );
        }
    }
    Ok(())
}

fn resolve_timeout(cfg: &Config, cli_timeout: Option<u64>) -> anyhow::Result<u64> {
    let raw = cli_timeout
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.timeout_raw());
    if raw.trim() == "0" {
        anyhow::bail!("Set QEMU_TIMEOUT (e.g., virtuoso test --timeout 60)");
    }
    raw.trim().parse().context("QEMU_TIMEOUT must be a number")
}

/// 单次完整测试，返回 (退出码, verdict 字符串)。
/// `accel_override` = None 时按平台规则（--tcg 语义由调用方折算）。
fn test_once(
    cfg: &Config,
    cli_arch: Option<&str>,
    timeout_secs: u64,
    accel_override: Option<Accel>,
    only: &[String],
) -> anyhow::Result<(i32, String)> {
    let arch = resolve_arch(cfg, cli_arch)?;
    let accel = match accel_override {
        Some(a) => a,
        None => resolve_accel(false, false, arch)?,
    };
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

    // ---- 启动（launcher；判定在 judge）----
    let started = Instant::now();
    let inv = build_invocation(&LaunchPlan {
        cfg,
        arch,
        topo: topo.clone(),
        accel,
        auto_test: cfg.auto_test(),
        only_tests: only.to_vec(),
        agent_socket: open_agent_socket(cfg, &run.path, "agent"),
    })?;
    println!("[LAUNCH] {}", inv.command_line()?);
    println!("Running QEMU test with {timeout_secs}s timeout...");
    let (mut child, mut sup) = inv.spawn_supervised(true)?;

    // 墙钟看门狗：到点 KILL 进程组（等价 timeout --signal=KILL 的 124 语义）
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog =
        launcher::spawn_watchdog(sup.pgid(), timeout_secs, Arc::clone(&timed_out));

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

    // 退出码对账（冻结契约 0=通过）：QEMU 正常关机恒 exit 0，guest 内失败
    // 只体现在 verdict（标记协议）——非 passed 折成 1；超时 124 / 中断 130
    // 原样保留。verdict.json 里的 exit_code 仍是 QEMU 原始码（审计事实）。
    let code = if code == 0 && verdict != "passed" { 1 } else { code };

    match code {
        0 => println!("Test completed successfully!"),
        124 => eprintln!("ERROR: Test timed out after {timeout_secs} seconds"),
        _ => {}
    }
    println!(
        "[RUN] verdict: {verdict} (exit {code}, {:.1}s) — verdict.json: {}",
        duration_ms as f64 / 1000.0,
        run.path.join("verdict.json").display()
    );
    Ok((code, verdict))
}

/// 构建失败时落盘的 verdict 元数据（build_failed 成档，退出码 1）。
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
