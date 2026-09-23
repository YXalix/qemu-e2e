//! parity 校验：同一 target 分别以 make 与 virtuoso 执行，比较退出码。
//! Makefile 是 xtask 的转发壳，双方退出码天然一致；
//! 本命令作为行为对照工具保留。

use std::process::Command;

use super::{dispatch, spawn_status};
use crate::config::Config;
use crate::{Command as CliCommand, SkillAction};

pub fn run_parity(target: &str, force: bool, strict: bool) -> anyhow::Result<i32> {
    let inner: CliCommand = match target {
        "verify" => CliCommand::Verify { arch: None },
        "initrd" => CliCommand::Build,
        "busybox" => CliCommand::BusyBox,
        "clean" => CliCommand::Clean,
        "qemu" => CliCommand::Shell { kvm: false },
        "qemu-kvm" => CliCommand::Shell { kvm: true },
        "qemu-debug" => CliCommand::Debug,
        "qemu-test" => CliCommand::Test {
            timeout: None,
            arch: None,
            replay_until_fail: None,
        },
        "install-skill" => CliCommand::Skill {
            action: SkillAction::Install,
        },
        "uninstall-skill" => CliCommand::Skill {
            action: SkillAction::Uninstall,
        },
        other => {
            eprintln!("ERROR: unsupported parity target `{other}`");
            eprintln!("  supported: verify initrd busybox clean qemu qemu-kvm qemu-debug qemu-test install-skill uninstall-skill");
            return Ok(2);
        }
    };

    let boots_vm = matches!(
        inner,
        CliCommand::Shell { .. } | CliCommand::Debug | CliCommand::Test { .. }
    );
    let heavy_build = matches!(inner, CliCommand::Build);
    if (boots_vm || heavy_build) && !force {
        eprintln!("ERROR: target `{target}` 会启动 VM 或执行 BusyBox 全量构建；确认后请加 --force");
        return Ok(2);
    }

    println!("[PARITY] target: {target}");

    let make_code = {
        let cfg = Config::load()?;
        let mut c = Command::new("make");
        c.arg("-C").arg(&cfg.project_root).arg(target);
        spawn_status(c)?
    };
    println!("[PARITY] make  exit: {make_code}");

    let xtask_code = dispatch(inner)?;
    println!("[PARITY] xtask exit: {xtask_code}");

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
