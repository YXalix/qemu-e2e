//! 构建与资产管理命令：build / busybox / disk / clean / skill。
//! 实际构建逻辑在 builder，本模块只做配置投影与进程接线。

use std::path::Path;
use std::process::Command;

use super::{resolve_arch, spawn_status};
use crate::config::Config;
use crate::SkillAction;

pub fn run_build() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    build_pair_for(&cfg, None).map(|_| 0)
}

/// 构建两段式引导对（test 路径以 run 目录 build.log 承载进度输出）。
pub(crate) fn build_pair_for(cfg: &Config, log_path: Option<&Path>) -> anyhow::Result<()> {
    let arch = resolve_arch(cfg, None)?;
    let (kernel_path, _) = cfg.kernel_path()?;
    let supply = cfg.busybox_supply();
    let mut progress = match log_path {
        Some(p) => builder::Progress::with_log(p)?,
        None => builder::Progress::stdout(),
    };
    progress.line("Rebuilding initrd.img + rootfs.img (two-stage boot pair)...");
    builder::build_boot_pair(&cfg.infra_dir, &kernel_path, arch, &supply, &[], &mut progress)
}

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

pub fn run_clean() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    for f in ["disk.qcow2", "initrd.img", "rootfs.img"] {
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

/// 确保当前 ARCH 的静态 BusyBox（四层供给链，builder 接管）。
pub fn run_busybox() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let arch = resolve_arch(&cfg, None)?;
    let supply = cfg.busybox_supply();
    let mut progress = builder::Progress::stdout();
    builder::busybox::ensure(&cfg.infra_dir, arch, &supply, &mut progress)?;
    Ok(0)
}

pub fn run_skill(action: SkillAction) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    // parity：make install-skill 要求 .env 显式设置 KERNEL_PATH（不自动探测）
    let Some(kernel_path) = cfg.env.get("KERNEL_PATH").filter(|s| !s.is_empty()) else {
        eprintln!("ERROR: KERNEL_PATH is not set.");
        eprintln!("  Copy .env.example to .env and set KERNEL_PATH to your kernel tree.");
        return Ok(1);
    };

    match action {
        SkillAction::Install => {
            println!("Installing kernel-dev + kernel-virtuoso skills for Claude Code...");
            for skill in ["kernel-dev", "kernel-virtuoso"] {
                let dst_dir = Path::new(&kernel_path).join(".claude/skills").join(skill);
                std::fs::create_dir_all(&dst_dir)?;
                let src = cfg.project_root.join("skills").join(skill).join("SKILL.md");
                std::fs::copy(&src, dst_dir.join("SKILL.md"))?;
                println!("  installed: {skill}");
            }
            println!(
                "Done. Claude Code will now recognize the skills when running from {kernel_path}/"
            );
            Ok(0)
        }
        SkillAction::Uninstall => {
            for skill in ["kernel-dev", "kernel-virtuoso"] {
                let dst = Path::new(&kernel_path).join(".claude/skills").join(skill);
                if dst.is_dir() {
                    std::fs::remove_dir_all(&dst)?;
                    println!("Removed: {}", dst.display());
                } else {
                    println!("Skill not installed at {}", dst.display());
                }
            }
            Ok(0)
        }
    }
}
