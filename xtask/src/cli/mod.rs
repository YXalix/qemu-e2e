//! CLI 编排：子命令分发与各命令组的接线。
//!
//! - verify  → `cli::verify`（前置检查 + firecracker 诊断）
//! - build   → `cli::build`（build / busybox / disk / clean / skill）
//! - vm      → `cli::vm`（shell / debug / test / matrix：启动、看门狗、判定接线）
//! - parity  → `cli::parity`（make ↔ xtask 行为对照）
//! - 呈现命令 → `runs::render`（triage / runs / cluster / suggest / replay）
//!
//! 行为基线（退出码语义）不变：0=通过、124=超时、其余=失败（单点在 judge::exit）。

mod build;
mod diagnostics;
mod parity;
mod verify;
mod vm;

use std::path::Path;
use std::process::Command;

use launcher::{Arch, Backend, NumaTopology};

use crate::config::Config;
use crate::runs;
use crate::Command as CliCommand;

pub(crate) fn code_of(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(130)
}

/// spawn 失败时模拟 shell 的 127（command not found），保持与 make 的退出码对齐。
pub(crate) fn spawn_status(mut cmd: Command) -> std::io::Result<i32> {
    cmd.status().map(code_of).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            std::io::Error::from_raw_os_error(127)
        } else {
            e
        }
    })
}

pub fn dispatch(cmd: CliCommand) -> anyhow::Result<i32> {
    match cmd {
        CliCommand::Verify { arch, backend } => verify::run_verify(arch.as_deref(), backend.as_deref()),
        CliCommand::Build => build::run_build(),
        CliCommand::Shell { kvm, backend } => vm::run_shell(kvm, backend.as_deref()),
        CliCommand::Debug => vm::run_debug(),
        CliCommand::Test { timeout, arch, replay_until_fail, backend } => {
            vm::run_test(timeout, arch.as_deref(), replay_until_fail, backend.as_deref())
        }
        CliCommand::Disk => build::run_disk(),
        CliCommand::BusyBox => build::run_busybox(),
        CliCommand::Clean => build::run_clean(),
        CliCommand::Skill { action } => build::run_skill(action),
        CliCommand::Matrix { arch } => vm::run_matrix(arch.as_deref()),
        CliCommand::Triage { run, json } => {
            let cfg = Config::load()?;
            runs::run_triage(&cfg, run.as_deref(), json)
        }
        CliCommand::Runs { json } => {
            let cfg = Config::load()?;
            runs::run_runs(&cfg, json)
        }
        CliCommand::Replay { log, json } => runs::run_replay(&log, json),
        CliCommand::Parity { target, force, strict } => parity::run_parity(&target, force, strict),
        CliCommand::Cluster { json } => {
            let cfg = Config::load()?;
            runs::run_cluster(&cfg, json)
        }
        CliCommand::Suggest { diff, json } => {
            let cfg = Config::load()?;
            runs::run_suggest(&cfg, diff, json)
        }
    }
}

// ---------------------------------------------------------------- 配置解析（各命令组共用）

pub(crate) fn resolve_arch(cfg: &Config, cli_arch: Option<&str>) -> anyhow::Result<Arch> {
    cli_arch
        .and_then(Arch::parse)
        .or_else(|| cfg.arch())
        .ok_or_else(|| anyhow::anyhow!("无法解析目标架构（检查 ARCH 值）"))
}

/// 解析 NUMA 拓扑（非法配置解析期报错，而非运行时）。
pub(crate) fn resolve_topology(cfg: &Config) -> anyhow::Result<NumaTopology> {
    NumaTopology::parse(
        &cfg.env.get("SMP").unwrap_or_else(|| "8".into()),
        &cfg.env.get("NUMA_NODES").unwrap_or_else(|| "1".into()),
        &cfg.env.get("NUMA_MEMORY").unwrap_or_else(|| "1G".into()),
    )
    .map_err(|e| {
        anyhow::anyhow!(
            "{e}\n  修改 .env 的 SMP / NUMA_NODES / NUMA_MEMORY（SMP 必须被 NUMA_NODES 整除）"
        )
    })
}

/// 解析启动后端：CLI --backend > BACKEND 配置 > 缺省 qemu。未知值一律报错。
pub(crate) fn resolve_backend(cfg: &Config, cli_backend: Option<&str>) -> anyhow::Result<Backend> {
    let raw = cli_backend
        .map(str::to_string)
        .or_else(|| cfg.env.get("BACKEND").filter(|s| !s.trim().is_empty()));
    match raw.as_deref() {
        None => Ok(Backend::Qemu),
        Some(s) => Backend::parse(s).ok_or_else(|| {
            anyhow::anyhow!("未知后端 `{s}`（允许: qemu | firecracker）")
        }),
    }
}

/// firecracker preflight（硬校验下沉 launcher::firecracker::preflight；
/// 本函数只把类型化配置投影为参数）。
pub(crate) fn firecracker_preflight(cfg: &Config, arch: Arch, kernel: &Path) -> anyhow::Result<()> {
    let (kernel_path, _) = cfg.kernel_path()?;
    launcher::firecracker::preflight(
        arch,
        kernel,
        &kernel_path.join(".config"),
        cfg.env.get("FIRECRACKER_BIN").as_deref().unwrap_or("firecracker"),
    )
}

/// firecracker 内核路径：x86_64 复用 bzImage；aarch64 需 ELF —— 优先
/// KERNEL_IMAGE 覆盖，否则回退内核树顶层 vmlinux。
pub(crate) fn firecracker_kernel(cfg: &Config, arch: Arch) -> anyhow::Result<std::path::PathBuf> {
    if let Some(p) = cfg.env.get("KERNEL_IMAGE").filter(|s| !s.is_empty()) {
        return Ok(std::path::PathBuf::from(p));
    }
    let (kernel_path, _) = cfg.kernel_path()?;
    Ok(match arch {
        Arch::Arm64 => kernel_path.join("vmlinux"),
        _ => kernel_path.join(arch.kernel_img()),
    })
}

pub(crate) fn kernel_image_path(cfg: &Config, arch: Arch) -> anyhow::Result<std::path::PathBuf> {
    let (kernel_path, _) = cfg.kernel_path()?;
    Ok(cfg
        .env
        .get("KERNEL_IMAGE")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| kernel_path.join(arch.kernel_img())))
}

pub(crate) fn disk_opt(cfg: &Config) -> Option<std::path::PathBuf> {
    let d = cfg.infra_dir.join("disk.qcow2");
    d.is_file().then_some(d)
}

/// QEMU_OPTS 透传（shell 展开语义：按空白切分）。
pub(crate) fn qemu_extra(cfg: &Config) -> Vec<String> {
    cfg.env
        .get("QEMU_OPTS")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}
