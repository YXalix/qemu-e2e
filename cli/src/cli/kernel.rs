//! `virtuoso kernel`：容器化内核供给（forge 命令投影）。
//!
//! 本模块只做配置投影与进程接线：容器工具链工作流（clone/defconfig/
//! menuconfig/build/cc*/export/shell）+ 卷管理与 current 切换（list/use），
//! 逻辑在 forge。产出 = 宿主可见内核树（`virtuoso kernel path`），kernel_path
//! 指向它即进入宿主原生测试主循环——verdict 管线不感知供给模式。

use std::path::PathBuf;

use launcher::Arch;

use forge::Progress;

use super::resolve_arch;
use crate::config::Config;
use crate::KernelAction;

pub fn run_kernel(action: KernelAction) -> anyhow::Result<i32> {
    match action {
        KernelAction::Clone {
            url,
            ref_name,
            as_volume,
            arch,
        } => run_clone(&url, ref_name.as_deref(), as_volume.as_deref(), arch.as_deref()),
        KernelAction::Defconfig { name, arch } => {
            run_defconfig(name.as_deref(), arch.as_deref())
        }
        KernelAction::Menuconfig { arch } => run_menuconfig(arch.as_deref()),
        KernelAction::Build { jobs, arch } => run_build(jobs, arch.as_deref()),
        KernelAction::Cc { arch } => run_cc(arch.as_deref()),
        KernelAction::Ccr { arch } => run_ccr(arch.as_deref()),
        KernelAction::Ccfix { arch } => run_ccfix(arch.as_deref()),
        KernelAction::Path { volume } => run_path(volume.as_deref()),
        KernelAction::Export { dest, arch } => run_export(dest.as_deref(), arch.as_deref()),
        KernelAction::Shell { arch } => run_shell(arch.as_deref()),
        KernelAction::List => run_list(),
        KernelAction::Use { volume, arch } => run_use(&volume, arch.as_deref()),
    }
}

// ---------------------------------------------------------------- 投影 helpers

/// 架构解析：CLI --arch > env KERNEL_ARCH > resolve_arch（顶层 arch > 宿主缺省）。
/// —— 测哪个架构就编哪个架构：缺省与 virtuoso.toml 的 arch 天然一致。
fn kernel_arch(cfg: &Config, cli_arch: Option<&str>) -> anyhow::Result<Arch> {
    if let Some(a) = cli_arch {
        return Arch::parse(a).ok_or_else(|| anyhow::anyhow!("未知架构 {a}"));
    }
    if let Ok(v) = std::env::var("KERNEL_ARCH") {
        if !v.trim().is_empty() {
            return Arch::parse(v.trim())
                .ok_or_else(|| anyhow::anyhow!("KERNEL_ARCH 非法: {v}（arm64|x86_64|riscv64）"));
        }
    }
    resolve_arch(cfg, None)
}

/// 活动卷解析：env KERNEL_VOLUME > 状态文件 current > 缺省卷。
fn current_volume(cfg: &Config) -> anyhow::Result<String> {
    if let Ok(v) = std::env::var("KERNEL_VOLUME") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    if let Some(cur) = forge::state::read(&cfg.project_root)? {
        return Ok(cur.volume);
    }
    Ok(forge::DEFAULT_VOLUME.to_string())
}

/// 工具链镜像：env KERNEL_TOOLCHAIN_IMAGE > ghcr 发布镜像（pull 失败回落本地
/// 构建 devkit/docker/Dockerfile.kernel）。
fn toolchain_image() -> String {
    std::env::var(forge::toolchain::IMAGE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| forge::DEFAULT_IMAGE.to_string())
}

/// clone ref：CLI --ref > env KERNEL_REF > 缺省 master。
fn clone_ref(cli_ref: Option<&str>) -> String {
    cli_ref
        .map(str::to_string)
        .or_else(|| std::env::var("KERNEL_REF").ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| forge::DEFAULT_REF.to_string())
}

fn dockerfile_dir(cfg: &Config) -> PathBuf {
    cfg.project_root.join("devkit").join("docker")
}

// ---------------------------------------------------------------- 工作流命令

fn run_clone(
    url: &str,
    cli_ref: Option<&str>,
    as_volume: Option<&str>,
    cli_arch: Option<&str>,
) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = as_volume
        .map(str::to_string)
        .or_else(|| std::env::var("KERNEL_VOLUME").ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| forge::DEFAULT_VOLUME.to_string());
    let arch = kernel_arch(&cfg, cli_arch)?;
    let ref_name = clone_ref(cli_ref);
    let mut progress = Progress::stdout();
    let view = forge::clone::run(
        &cfg.project_root,
        &volume,
        arch,
        url,
        &ref_name,
        &toolchain_image(),
        &dockerfile_dir(&cfg),
        &mut progress,
    )?;
    println!("Kernel: cloned {url}@{ref_name} → volume {volume}（.clangd 已按 {} 配好）", arch.name());
    println!("Kernel: current → {volume} ({})", arch.name());
    println!("宿主可见路径（AI/编辑器 cwd）：{}", view.display());
    println!("next: virtuoso kernel defconfig && virtuoso kernel build");
    Ok(0)
}

fn run_defconfig(name: Option<&str>, cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    let arch = kernel_arch(&cfg, cli_arch)?;
    let target = name.unwrap_or("defconfig");
    println!("Kernel: make {target}（volume {volume}，arch {}）", arch.name());
    forge::toolchain::run_streaming(
        &volume,
        &toolchain_image(),
        None,
        &forge::toolchain::make_env(arch),
        &format!("make {}", forge::toolchain::shell_quote(target)),
    )?;
    Ok(0)
}

fn run_menuconfig(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    let arch = kernel_arch(&cfg, cli_arch)?;
    forge::toolchain::run_tty(
        &volume,
        &toolchain_image(),
        &forge::toolchain::make_env(arch),
        &["make", "menuconfig"],
    )
}

fn run_build(jobs: Option<usize>, cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    let arch = kernel_arch(&cfg, cli_arch)?;
    let jobs = jobs.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8));
    let image_target = forge::toolchain::make_image_target(arch);
    println!(
        "Kernel: make -j{jobs} {image_target} modules（volume {volume}，arch {}）",
        arch.name()
    );
    forge::toolchain::run_streaming(
        &volume,
        &toolchain_image(),
        None,
        &forge::toolchain::make_env(arch),
        &format!("make -j{jobs} {image_target} modules"),
    )?;
    forge::toolchain::cdb_generate(&volume, &toolchain_image())?;
    let view = forge::volume::host_view(&volume)?;
    forge::toolchain::cdb_host_form(&volume, &toolchain_image(), &view)?;
    println!(
        "Kernel: built {image_target} + modules；compile_commands.json 已按宿主路径改写（宿主 clangd 直接消费）"
    );
    Ok(0)
}

fn run_cc(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    kernel_arch(&cfg, cli_arch)?; // 校验 arch 解析（CDB 渲染不依赖，但保持命令语义一致）
    let image = toolchain_image();
    forge::toolchain::cdb_generate(&volume, &image)?;
    let view = forge::volume::host_view(&volume)?;
    forge::toolchain::cdb_host_form(&volume, &image, &view)?;
    println!("Kernel: compile_commands.json 已生成（宿主路径形态）");
    Ok(0)
}

fn run_ccr(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    kernel_arch(&cfg, cli_arch)?;
    forge::toolchain::cdb_generate(&volume, &toolchain_image())?;
    println!("Kernel: compile_commands.json 已生成（/ksrc 原始形态，devcontainer 消费）");
    Ok(0)
}

fn run_ccfix(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    kernel_arch(&cfg, cli_arch)?;
    let view = forge::volume::host_view(&volume)?;
    forge::toolchain::cdb_host_form(&volume, &toolchain_image(), &view)?;
    println!("Kernel: compile_commands.json 已原地改写为宿主路径形态");
    Ok(0)
}

fn run_path(volume_flag: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = match volume_flag {
        Some(v) => v.to_string(),
        None => current_volume(&cfg)?,
    };
    let view = forge::volume::host_view(&volume)?;
    if !view.is_dir() {
        if std::env::consts::OS == "macos" {
            anyhow::bail!(
                "{} 不可达——OrbStack 未安装或未运行（视图仅在 OrbStack 运行时存在）。\n  启动 OrbStack 后重试；或 `virtuoso kernel export` 走最小树回退（kernel_path 指 target/kernel/<arch>）。",
                view.display()
            );
        }
        anyhow::bail!("volume {volume} 不存在或不可达——先 `virtuoso kernel clone <git-url>`。");
    }
    println!("{}", view.display());
    Ok(0)
}

fn run_export(dest: Option<&str>, cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    let arch = kernel_arch(&cfg, cli_arch)?;
    let out_dir = cfg.target_dir.join("kernel");
    std::fs::create_dir_all(&out_dir)?;
    let dest = dest.unwrap_or(arch.name());
    let img = arch.kernel_img();
    let img_dir = std::path::Path::new(img)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");
    let q = forge::toolchain::shell_quote;
    println!(
        "Kernel: exporting minimal tree（volume {volume} → target/kernel/{dest}，arch {}）",
        arch.name()
    );
    forge::toolchain::run_streaming(
        &volume,
        &toolchain_image(),
        Some(&out_dir),
        &forge::toolchain::make_env(arch),
        &format!(
            "make INSTALL_MOD_PATH=/tmp/mods modules_install >/dev/null && \
             rm -rf {d} && mkdir -p {d}/{img_dir} {d} && \
             cp Makefile .config {d}/ && cp {img} {d}/{img} && \
             cp -a /tmp/mods/lib {d}/lib",
            d = q(dest),
        ),
    )?;
    println!("Kernel: exported minimal tree → target/kernel/{dest}");
    println!("  kernel_path 指 target/kernel/{dest}（或 KERNEL_PATH env）即可跑 virtuoso 主循环");
    Ok(0)
}

fn run_shell(cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volume = current_volume(&cfg)?;
    let arch = kernel_arch(&cfg, cli_arch)?;
    forge::toolchain::run_tty(
        &volume,
        &toolchain_image(),
        &forge::toolchain::make_env(arch),
        &["/bin/bash"],
    )
}

// ---------------------------------------------------------------- 卷管理命令

fn run_list() -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    let volumes = forge::volume::list()?;
    if volumes.is_empty() {
        println!("（引擎里还没有卷——`virtuoso kernel clone <git-url>` 创建）");
        return Ok(0);
    }
    let current = forge::state::read(&cfg.project_root)?;
    for v in volumes {
        let view = forge::volume::host_view(&v).ok();
        let label = match view.as_ref().map(|p| forge::volume::status(p)) {
            Some(s) => s.label(),
            None => "unreachable",
        };
        let mark = if current.as_ref().is_some_and(|c| c.volume == v) {
            "*"
        } else {
            " "
        };
        let note = match current.as_ref() {
            Some(c) if c.volume == v => format!("← current (arch {})", c.arch),
            _ => String::new(),
        };
        println!("{mark} {v:<32} {label:<12} {}", note);
    }
    Ok(0)
}

fn run_use(volume: &str, cli_arch: Option<&str>) -> anyhow::Result<i32> {
    let cfg = Config::load()?;
    anyhow::ensure!(
        forge::volume::exists(volume),
        "volume {volume} 不存在（`virtuoso kernel list` 查看已有卷，或 `virtuoso kernel clone <git-url> --as {volume}` 创建）"
    );
    let arch = match cli_arch {
        Some(a) => Arch::parse(a).ok_or_else(|| anyhow::anyhow!("未知架构 {a}"))?,
        None => kernel_arch(&cfg, None)?,
    };
    forge::state::write(
        &cfg.project_root,
        &forge::state::Current {
            volume: volume.to_string(),
            arch: arch.name().to_string(),
        },
    )?;
    println!("Kernel: current → {volume} ({})", arch.name());
    println!("next: virtuoso kernel path  # 宿主可见路径，指给 kernel_path / AI cwd");
    Ok(0)
}
