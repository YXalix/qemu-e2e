//! `virtuoso fetch`：拉取 preset 预编内核（mainline mini Image，全 =y 零
//! 模块）到 target/kernel/preset，供 kernel_preset 配置开箱即用——不装内核
//! 树也能 test。供给层在 builder::preset（版本解析/下载/SHA256 校验）。

use launcher::Arch;

use super::{preset_dir, preset_kind, resolve_arch};
use crate::config::Config;

pub fn run_fetch(version: Option<&str>, arch_cli: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let preset = preset_kind(&cfg)?;
    anyhow::ensure!(
        preset.is_some(),
        "kernel_preset 未启用：先在 virtuoso.toml 设 kernel_preset = \"mainline\"（或 KERNEL_PRESET env）再 fetch"
    );

    // 缺省三架构全量（matrix 与 CI e2e 直接可用）；--arch 裁剪
    let arches: Vec<Arch> = match arch_cli {
        Some(a) => vec![resolve_arch(&cfg, Some(a))?],
        None => vec![Arch::Arm64, Arch::X86_64, Arch::Riscv64],
    };

    let repo = builder::preset::release_repo(
        std::env::var("KERNEL_RELEASE_REPO").ok().as_deref(),
        &cfg.project_root,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "无法确定 preset 内核 release 仓库：设 KERNEL_RELEASE_REPO=owner/repo，或让 git origin 指向 GitHub"
        )
    })?;

    // 版本解析：--version > infra/kernel/pin 钉定 > 最新已发布
    let version = match version.map(builder::preset::normalize_version) {
        Some(Some(v)) => v,
        Some(None) => anyhow::bail!("非法版本号：{version:?}（期望 X.Y[.Z]）"),
        None => match builder::preset::pin_version(&cfg.infra_dir) {
            Some(v) => v,
            None => builder::preset::latest_released(&repo)?,
        },
    };

    let dir = preset_dir(&cfg);
    let mut progress = builder::Progress::stdout();
    let mut got = Vec::new();
    for arch in arches {
        let p = builder::preset::ensure_image(&repo, &version, &dir, arch, &mut progress)?;
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        got.push(format!(
            "{} {}",
            arch.name(),
            common::fmt::human_size_ls(size)
        ));
    }
    std::fs::write(dir.join("version"), &version)?;
    println!(
        "Kernel preset: mainline v{version} → {} ({})",
        dir.display(),
        got.join(" · ")
    );
    let hint = match cfg.timeout_raw() {
        t if t.trim() == "0" => "virtuoso test --timeout 300".to_string(),
        t => format!("virtuoso test --timeout {}", t.trim()),
    };
    println!("next: {hint}");
    Ok(0)
}
