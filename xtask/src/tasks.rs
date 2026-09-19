//! Phase 1 任务实现：对 qemu-e2e 8 个 make target 的**对等包装**。
//!
//! 原则：QEMU/构建的实际行为仍由 `infra/*.sh` 承担（单一行为基线），
//! xtask 只做进程编排、CLI 透传与退出码对齐。Phase 2 再逐模块 Rust 化。

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::config::Config;
use crate::Command as CliCommand;

/// 当前被包装的 QEMU 进程组（供 Ctrl-C 守卫收割）。
static ACTIVE_PGID: AtomicU32 = AtomicU32::new(0);

pub fn install_ctrlc_guard() {
    let _ = ctrlc::set_handler(|| {
        let pgid = ACTIVE_PGID.load(Ordering::SeqCst);
        if pgid != 0 {
            kill_group(pgid);
        }
        std::process::exit(130);
    });
}

fn kill_group(pgid: u32) {
    let _ = Command::new("bash")
        .arg("-c")
        .arg(format!("kill -KILL -- -{pgid} 2>/dev/null || true"))
        .status();
}

fn code_of(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(130)
}

/// spawn 失败时模拟 shell 的 127（command not found），保持与 make 的退出码对齐。
fn spawn_status(mut cmd: Command) -> std::io::Result<i32> {
    cmd.status().map(|s| code_of(s)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            std::io::Error::from_raw_os_error(127)
        } else {
            e
        }
    })
}

pub fn dispatch(cmd: CliCommand) -> anyhow::Result<i32> {
    match cmd {
        CliCommand::Verify { arch } => run_verify(arch.as_deref()),
        CliCommand::Build => run_build(),
        CliCommand::Shell { kvm } => run_shell(kvm),
        CliCommand::Debug => run_debug(),
        CliCommand::Test { timeout, arch } => run_test(timeout, arch.as_deref()),
        CliCommand::Disk => run_disk(),
        CliCommand::BusyBox => run_busybox(),
        CliCommand::Clean => run_clean(),
        CliCommand::Skill { action } => run_skill(action),
        CliCommand::Matrix { .. } => {
            eprintln!("NOT IMPLEMENTED: matrix 属于 Phase 2（ensemble 多架构矩阵）。");
            eprintln!("当前等价操作: for a in arm64 x86_64 riscv64; do cargo xtask test --arch $a; done");
            Ok(2)
        }
        CliCommand::Triage => {
            eprintln!("NOT IMPLEMENTED: triage 属于 Phase 2（auditor events.jsonl 数据源就绪后开放）。");
            Ok(2)
        }
        CliCommand::Replay { .. } => {
            eprintln!("NOT IMPLEMENTED: replay 属于 Phase 2（encore 串口日志回放）。");
            Ok(2)
        }
        CliCommand::Parity { target, force, strict } => run_parity(&target, force, strict),
    }
}

// ---------------------------------------------------------------- verify

pub fn run_verify(arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    cfg.print_diagnostics(arch);
    let mut cmd = Command::new("./verify.sh");
    cmd.current_dir(&cfg.infra_dir);
    if let Some(a) = arch {
        cmd.env("ARCH", a);
    }
    Ok(spawn_status(cmd)?)
}

// ---------------------------------------------------------------- build (initrd)

pub fn run_build() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    println!("Rebuilding initrd.img...");
    let mut cmd = Command::new("./build-initrd.sh");
    cmd.current_dir(&cfg.infra_dir);
    Ok(spawn_status(cmd)?)
}

// ---------------------------------------------------------------- shell / debug

pub fn run_shell(kvm: bool) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    if kvm {
        println!("Starting QEMU with KVM acceleration...");
    } else {
        println!("Starting QEMU...");
    }
    let mut cmd = Command::new("./run-qemu.sh");
    cmd.current_dir(&cfg.infra_dir);
    if let Some(img) = cfg.env.get("KERNEL_IMAGE").filter(|s| !s.is_empty()) {
        cmd.arg(img);
    }
    if kvm {
        cmd.env("QEMU_KVM", "1");
    }
    Ok(spawn_status(cmd)?)
}

pub fn run_debug() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    println!("Starting QEMU with GDB stub on port 1234...");
    let mut cmd = Command::new("./run-qemu.sh");
    cmd.current_dir(&cfg.infra_dir);
    if let Some(img) = cfg.env.get("KERNEL_IMAGE").filter(|s| !s.is_empty()) {
        cmd.arg(img);
    }
    cmd.env("QEMU_DEBUG", "1");
    Ok(spawn_status(cmd)?)
}

// ---------------------------------------------------------------- test (qemu-test)

/// make qemu-test 对等：
/// 1. QEMU_TIMEOUT=0（或未设置）拒绝执行；
/// 2. 先重建 initrd；
/// 3. `timeout --signal=KILL <n>` 包裹 run-qemu.sh，AUTO_TEST=1；
/// 4. 退出码 124/137 归一为 124 并收割进程组。
pub fn run_test(cli_timeout: Option<u64>, cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let timeout_s = cli_timeout
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.timeout_raw());
    if timeout_s.trim() == "0" {
        eprintln!("ERROR: Set QEMU_TIMEOUT (e.g., cargo xtask test --timeout 60)");
        return Ok(1);
    }

    let code = run_build()?;
    if code != 0 {
        return Ok(code);
    }

    let image = cfg.env.get("KERNEL_IMAGE").filter(|s| !s.is_empty());
    let mut inner = String::from("exec ./run-qemu.sh");
    if let Some(img) = &image {
        inner.push_str(&format!(" '{img}'"));
    }

    println!("Running QEMU test with {timeout_s}s timeout...");
    let mut cmd = Command::new("timeout");
    cmd.args(["--signal=KILL", timeout_s.trim(), "bash", "-c", &inner])
        .current_dir(&cfg.infra_dir)
        .env("AUTO_TEST", cfg.auto_test());
    if let Some(a) = cli_arch {
        cmd.env("ARCH", a);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn()?;
    let pgid = child.id();
    ACTIVE_PGID.store(pgid, Ordering::SeqCst);

    let result = child.wait();
    ACTIVE_PGID.store(0, Ordering::SeqCst);
    let status = result?;

    match status.code() {
        Some(0) => {
            println!("Test completed successfully!");
            Ok(0)
        }
        Some(124) | Some(137) => {
            eprintln!("ERROR: Test timed out after {} seconds", timeout_s.trim());
            kill_group(pgid);
            Ok(124)
        }
        Some(n) => Ok(n),
        None => {
            kill_group(pgid);
            Ok(130)
        }
    }
}

// ---------------------------------------------------------------- disk

pub fn run_disk() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let disk = cfg.infra_dir.join("disk.qcow2");
    if disk.is_file() {
        println!("disk.qcow2 already exists.");
        return Ok(0);
    }
    println!("Creating disk.qcow2 (512MB block device)");
    let mut cmd = Command::new("qemu-img");
    cmd.args(["create", "-f", "qcow2"])
        .arg(&disk)
        .arg("512M")
        .current_dir(&cfg.infra_dir);
    Ok(spawn_status(cmd)?)
}

// ---------------------------------------------------------------- clean

pub fn run_clean() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    for f in ["disk.qcow2", "initrd.img"] {
        let p = cfg.infra_dir.join(f);
        if p.is_file() {
            std::fs::remove_file(&p)?;
        }
    }
    let build = cfg.infra_dir.join("testcases/build");
    if build.is_dir() {
        std::fs::remove_dir_all(&build)?;
    }
    Ok(0)
}

// ---------------------------------------------------------------- busybox

/// make busybox 对等：确保 ARCH 对应的静态 BusyBox（release 下载优先，源码兜底）
pub fn run_busybox() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    println!("Ensuring per-arch static BusyBox (release download / source fallback)...");
    let mut cmd = Command::new("./fetch-busybox.sh");
    cmd.current_dir(&cfg.infra_dir);
    Ok(spawn_status(cmd)?)
}

// ---------------------------------------------------------------- skill

pub fn run_skill(action: crate::SkillAction) -> anyhow::Result<i32> {
    use crate::SkillAction;
    let cfg = Config::load()?;
    // parity：make install-skill 要求 .env 显式设置 KERNEL_PATH（不自动探测）
    let Some(kernel_path) = cfg.env.get("KERNEL_PATH").filter(|s| !s.is_empty()) else {
        eprintln!("ERROR: KERNEL_PATH is not set.");
        eprintln!("  Copy .env.example to .env and set KERNEL_PATH to your kernel tree.");
        return Ok(1);
    };

    match action {
        SkillAction::Install => {
            println!("Installing kernel-dev skill for Claude Code...");
            let dst_dir = Path::new(&kernel_path)
                .join(".claude/skills/kernel-dev");
            std::fs::create_dir_all(&dst_dir)?;
            let src = cfg.project_root.join("skills/kernel-dev/SKILL.md");
            std::fs::copy(&src, dst_dir.join("SKILL.md"))?;
            println!(
                "Done. Claude Code will now recognize the kernel-dev skill when running from {kernel_path}/"
            );
            Ok(0)
        }
        SkillAction::Uninstall => {
            let dst = Path::new(&kernel_path).join(".claude/skills/kernel-dev");
            if dst.is_dir() {
                std::fs::remove_dir_all(&dst)?;
                println!("Removed: {}", dst.display());
            } else {
                println!("Skill not installed at {}", dst.display());
            }
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------- parity

/// Phase 1 验收工具：同一 target 分别以 make 与 cargo xtask 执行，比较退出码。
fn run_parity(target: &str, force: bool, strict: bool) -> anyhow::Result<i32> {
    use crate::SkillAction;
    let cfg = Config::load()?;

    let inner: CliCommand = match target {
        "verify" => CliCommand::Verify { arch: None },
        "initrd" => CliCommand::Build,
        "disk" => CliCommand::Disk,
        "busybox" => CliCommand::BusyBox,
        "clean" => CliCommand::Clean,
        "qemu" => CliCommand::Shell { kvm: false },
        "qemu-kvm" => CliCommand::Shell { kvm: true },
        "qemu-debug" => CliCommand::Debug,
        "qemu-test" => CliCommand::Test { timeout: None, arch: None },
        "install-skill" => CliCommand::Skill { action: SkillAction::Install },
        "uninstall-skill" => CliCommand::Skill { action: SkillAction::Uninstall },
        other => {
            eprintln!("ERROR: unsupported parity target `{other}`");
            eprintln!("  supported: verify initrd disk busybox clean qemu qemu-kvm qemu-debug qemu-test install-skill uninstall-skill");
            return Ok(2);
        }
    };

    let boots_vm = matches!(
        inner,
        CliCommand::Shell { .. } | CliCommand::Debug | CliCommand::Test { .. }
    );
    let heavy_build = matches!(inner, CliCommand::Build | CliCommand::BusyBox);
    if (boots_vm || heavy_build) && !force {
        eprintln!("ERROR: target `{target}` 会启动 VM 或执行 BusyBox 全量构建；确认后请加 --force");
        return Ok(2);
    }

    println!("[PARITY] target: {target}");

    let make_code = {
        let mut c = Command::new("make");
        c.arg("-C").arg(&cfg.project_root).arg(target);
        spawn_status(c)?
    };
    println!("[PARITY] make  exit: {make_code}");

    let xtask_code = dispatch(inner)?;
    println!("[PARITY] xtask exit: {xtask_code}");

    // 判定规则：
    //   PASS       —— 退出码严格相等；
    //   PASS (CI)  —— 双方均非零且不涉及超时类（124）。
    //     依据：GNU make 会把脚本失败折叠为其自身的退出码 2，并吞掉 `exit 124`；
    //     xtask 有意保留脚本真实退出码与 124 超时码（设计文档 §7 冻结语义），
    //     对 CI 的 "0 / 非零 / 124" 三态判定完全等价。--strict 可强制严格相等。
    let timeout_involved = make_code == 124 || xtask_code == 124;
    let (verdict, reason, code) = if make_code == xtask_code {
        ("[PASS]", "identical exit codes", 0)
    } else if !strict && make_code != 0 && xtask_code != 0 && !timeout_involved {
        (
            "[PASS]",
            "CI-equivalent (both non-zero; make folds script failures to 2, xtask preserves raw codes)",
            0,
        )
    } else {
        (
            "[FAIL]",
            "exit-code semantics differ (0 vs non-zero, or timeout-class mismatch)",
            1,
        )
    };
    println!("{verdict} parity ({target}): {reason}");
    Ok(code)
}
