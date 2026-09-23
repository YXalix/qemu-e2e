//! CLI 编排：子命令分发与各命令组的接线。
//!
//! - verify  → `cli::verify`（前置检查 + firecracker 诊断；引擎投影供 doctor 共用）
//! - doctor  → `cli::doctor`（verify 的 flutter-doctor 风格一屏简化呈现）
//! - build   → `cli::build`（build / busybox / clean / skill）
//! - vm      → `cli::vm`（shell / debug / test / matrix：启动、看门狗、判定接线）
//! - parity  → `cli::parity`（make ↔ xtask 行为对照）
//! - docs    → `cli::docs`（mdBook 文档构建 / 本地预览）
//! - 呈现命令 → `runs::render`（triage / runs / cluster / suggest / replay）
//!
//! 行为基线（退出码语义）不变：0=通过、124=超时、其余=失败（单点在 judge::exit）。

mod build;
mod diagnostics;
mod doctor;
mod docs;
mod parity;
mod probe;
mod verify;
mod vm;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
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
        CliCommand::Verify { arch, backend } => {
            verify::run_verify(arch.as_deref(), backend.as_deref())
        }
        CliCommand::Doctor {
            arch,
            backend,
            json,
        } => doctor::run_doctor(arch.as_deref(), backend.as_deref(), json),
        CliCommand::Build => build::run_build(),
        CliCommand::Shell { kvm, backend } => vm::run_shell(kvm, backend.as_deref()),
        CliCommand::Debug => vm::run_debug(),
        CliCommand::Test {
            timeout,
            arch,
            replay_until_fail,
            backend,
        } => vm::run_test(
            timeout,
            arch.as_deref(),
            replay_until_fail,
            backend.as_deref(),
        ),
        CliCommand::BusyBox => build::run_busybox(),
        CliCommand::Clean => build::run_clean(),
        CliCommand::Skill { action } => build::run_skill(action),
        CliCommand::Matrix { arch } => vm::run_matrix(arch.as_deref()),
        CliCommand::Probe {
            arch,
            timeout,
            cmds,
            cmd_file,
            json,
        } => probe::run_probe(arch.as_deref(), timeout, &cmds, cmd_file.as_deref(), json),
        CliCommand::Triage { run, json } => {
            let cfg = Config::load()?;
            runs::run_triage(&cfg, run.as_deref(), json)
        }
        CliCommand::Runs { json } => {
            let cfg = Config::load()?;
            runs::run_runs(&cfg, json)
        }
        CliCommand::Replay { log, json } => runs::run_replay(&log, json),
        CliCommand::Parity {
            target,
            force,
            strict,
        } => parity::run_parity(&target, force, strict),
        CliCommand::Cluster { json } => {
            let cfg = Config::load()?;
            runs::run_cluster(&cfg, json)
        }
        CliCommand::Suggest { diff, json } => {
            let cfg = Config::load()?;
            runs::run_suggest(&cfg, diff, json)
        }
        CliCommand::Docs { serve, open } => docs::run_docs(serve, open),
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
    let (smp, nodes, mem) = cfg.topo_params();
    NumaTopology::parse(&smp, &nodes, &mem).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n  修改 virtuoso.toml 的 smp / [components.numa]（SMP 必须被 NUMA 节点数整除）"
        )
    })
}

/// 解析启动后端：CLI --backend > backend 配置 > 缺省 qemu。未知值一律报错。
pub(crate) fn resolve_backend(cfg: &Config, cli_backend: Option<&str>) -> anyhow::Result<Backend> {
    let raw = cli_backend.map(str::to_string).or_else(|| cfg.backend_str());
    match raw.as_deref() {
        None => Ok(Backend::Qemu),
        Some(s) => Backend::parse(s)
            .ok_or_else(|| anyhow::anyhow!("未知后端 `{s}`（允许: qemu | firecracker）")),
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
        &cfg.firecracker_bin(),
    )
}

/// firecracker 内核路径：x86_64 复用 bzImage；aarch64 需 ELF —— 优先
/// kernel_image 覆盖，否则回退内核树顶层 vmlinux。
pub(crate) fn firecracker_kernel(cfg: &Config, arch: Arch) -> anyhow::Result<std::path::PathBuf> {
    if let Some(p) = cfg.kernel_image() {
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
        .kernel_image()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| kernel_path.join(arch.kernel_img())))
}

/// tools.img 数据盘（tools 外挂 virtio-blk → guest 内 /dev/vdb 挂 /tools）。
/// [components.tools_disk] enabled（段缺省 = true）且产物存在才附加——
/// DSL 缺省 argv 保持基线（冻结不变量 3）。
pub(crate) fn tools_disk_opt(cfg: &Config) -> Option<launcher::DataDisk> {
    if !cfg.tools_disk_enabled() {
        return None;
    }
    let p = cfg.artifacts_dir.join("tools.img");
    p.is_file().then(|| launcher::DataDisk::new("tools", p))
}

/// agent 通道 socket（[components.agent] enabled 才 Some）。
/// socket 落调用方给定的目录（shell 用 target/，test 用 run 目录）。
pub(crate) fn agent_socket_opt(cfg: &Config, dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    cfg.agent_enabled().then(|| dir.join(format!("{name}.sock")))
}

/// pmem 持久内存（[components.pmem] enabled 才 Some）：DT 途径
/// （arm64/riscv64 直启无 ACPI，QEMU nvdimm 的 NFIT 需 EFI 引导才可见，
/// 故用 pmem-region 设备树节点绕开）。产线三件套落 target/build/pmem/：
/// 1) ram.img —— 主内存后端文件（memory-backend-file share=on），挖出的
///    pmem 区即宿主文件 backed，guest 写入持久落盘；
/// 2) <machine>.dtb —— dumpdtb（按当前 -smp/-m 生成）+ fdtput 补丁（根节点
///    注入 pmem-region@<base>），经 -dtb 传入替换 QEMU 生成版；
/// 3) mem_limit —— cmdline 追加 `mem=<总内存 − pmem 区>`：QEMU 会按 -m
///    全量重建 /memory（无法缩 DTB），故由内核侧把 pmem 区排除出线性
///    内存模型（等价 x86 memmap= 语义），devm_memremap_pages 才能建
///    ZONE_DEVICE struct page（否则 sparse subsection 已存在 → -EEXIST）。
pub(crate) fn pmem_opt(
    cfg: &Config,
    arch: Arch,
    topo: &NumaTopology,
) -> anyhow::Result<Option<launcher::PmemSpec>> {
    let Some(size) = cfg.pmem_size() else {
        return Ok(None);
    };
    if arch == Arch::X86_64 {
        anyhow::bail!(
            "[components.pmem] x86_64 暂不支持（QEMU nvdimm 走 ACPI NFIT，需 EFI 引导）—— 仅 DT 架构 arm64/riscv64"
        );
    }
    let total_mem = topo.total_memory().map_err(anyhow::Error::msg)?;
    let pmem_bytes = memory_bytes(&size, "size")?;
    let total_bytes = memory_bytes(&total_mem, "-m 总内存")?;
    if pmem_bytes >= total_bytes {
        anyhow::bail!("[components.pmem] size ({size}) 必须小于总内存 ({total_mem})");
    }
    let limit_bytes = total_bytes - pmem_bytes;
    let mem_limit = render_memory(limit_bytes);

    let dir = cfg.build_dir.join("pmem");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建 {} 失败", dir.display()))?;
    let ram_backend = dir.join("ram.img");
    ensure_sparse(&ram_backend, total_bytes)?;
    let dtb = patch_pmem_dtb(cfg, arch, topo, &total_mem, &dir, pmem_bytes)?;
    Ok(Some(launcher::PmemSpec::new(
        size, mem_limit, ram_backend, dtb,
    )))
}

/// 字节数 → "768M"（MiB 整除时）/ 裸数字（QEMU 与内核 mem= 均按字节解析）。
fn render_memory(bytes: u64) -> String {
    if bytes != 0 && bytes.is_multiple_of(1024 * 1024) {
        format!("{}M", bytes / (1024 * 1024))
    } else {
        bytes.to_string()
    }
}

/// "256M"/裸数字 → 字节数（QEMU 语义：裸数字 = 字节）。
fn memory_bytes(s: &str, what: &str) -> anyhow::Result<u64> {
    let (n, unit) = common::units::parse_memory(s)
        .map_err(|e| anyhow::anyhow!("[components.pmem] {what} 非法: {e}"))?;
    Ok(match unit {
        common::units::MemUnit::Bare => n,
        _ => unit.to_mib(n) * 1024 * 1024,
    })
}

/// 文件缺失或大小不符才重建（稀疏）；已有内容保留 —— guest 写入跨 run 持久。
fn ensure_sparse(path: &Path, bytes: u64) -> anyhow::Result<()> {
    let mismatch = std::fs::metadata(path)
        .map(|m| m.len() != bytes)
        .unwrap_or(true);
    if mismatch {
        std::fs::File::create(path)
            .with_context(|| format!("创建 {} 失败", path.display()))?
            .set_len(bytes)
            .with_context(|| format!("设置 {} 大小为 {bytes} 字节失败", path.display()))?;
    }
    Ok(())
}

/// 32 位 DT cell 序（高字在前）→ fdtput -t x 的十六进制参数表。
fn dt_cells(v: u64, cells: u64) -> Vec<String> {
    (0..cells)
        .rev()
        .map(|i| format!("{:#x}", (v >> (32 * i)) & 0xffff_ffff))
        .collect()
}

fn fdtget_words(dtb: &Path, node: &str, prop: &str) -> anyhow::Result<Vec<u64>> {
    let out = Command::new("fdtget")
        .args(["-t", "x"])
        .arg(dtb)
        .arg(node)
        .arg(prop)
        .output()
        .with_context(|| format!("运行 fdtget 读取 {node} {prop} 失败（宿主需要 dtc 包）"))?;
    anyhow::ensure!(
        out.status.success(),
        "fdtget {node} {prop} 失败: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(|w| u64::from_str_radix(w.trim_start_matches("0x"), 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("fdtget {node} {prop} 输出解析失败: {e}"))
}

fn fdtput(dtb: &Path, args: &[String]) -> anyhow::Result<()> {
    let out = Command::new("fdtput")
        .arg(dtb)
        .args(args)
        .output()
        .with_context(|| format!("运行 fdtput {args:?} 失败（宿主需要 dtc 包）"))?;
    anyhow::ensure!(
        out.status.success(),
        "fdtput {args:?} 失败: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

/// dumpdtb 生成 QEMU 设备树并打 pmem 补丁：根节点注入 pmem-region@<base>
/// （of_pmem 绑定 → 免 ndctl /dev/pmem0）。区间排除出线性内存由 cmdline
/// `mem=` 完成（QEMU 对 -dtb 会按 -m 全量重建 /memory，DTB 缩减无效）。
fn patch_pmem_dtb(
    cfg: &Config,
    arch: Arch,
    topo: &NumaTopology,
    total_mem: &str,
    dir: &Path,
    pmem_bytes: u64,
) -> anyhow::Result<PathBuf> {
    let machine = arch.machine();
    let dtb = dir.join(format!("{machine}.dtb"));
    let qemu = cfg
        .qemu_override()
        .unwrap_or_else(|| arch.qemu_bin().to_string());
    let mut cmd = Command::new(&qemu);
    cmd.arg("-machine").arg(format!("{machine},dumpdtb={}", dtb.display()));
    cmd.args(["-smp", &topo.smp.to_string(), "-m", total_mem, "-display", "none"]);
    if arch == Arch::Riscv64 {
        cmd.arg("-bios").arg("none");
    }
    let out = cmd
        .output()
        .with_context(|| format!("运行 {qemu} dumpdtb 失败"))?;
    anyhow::ensure!(
        out.status.success() && dtb.is_file(),
        "dumpdtb 失败: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );

    // 根节点 cell 宽度（QEMU virt 为 #address-cells=2 / #size-cells=2）
    let read_cells = |prop: &str| -> anyhow::Result<u64> {
        Ok(fdtget_words(&dtb, "/", prop)?
            .first()
            .copied()
            .unwrap_or(2))
    };
    let addr_cells = read_cells("#address-cells")?;
    let size_cells = read_cells("#size-cells")?;

    // pmem 区固定在 RAM 顶部：dumpdtb 的 /memory reg 给出 RAM 基址/大小
    // （生成参数与启动一致；QEMU 启动时的 /memory 回写与补丁节点无关）
    let reg = fdtget_words(&dtb, "/memory", "reg")?;
    anyhow::ensure!(
        reg.len() == (addr_cells + size_cells) as usize,
        "/memory reg 形态非预期（{} 个 cell，期望 {}）",
        reg.len(),
        addr_cells + size_cells
    );
    let join = |words: &[u64]| -> u64 {
        words
            .iter()
            .fold(0u64, |acc, w| (acc << 32) | (w & 0xffff_ffff))
    };
    let base = join(&reg[..addr_cells as usize]);
    let total_bytes = join(&reg[addr_cells as usize..]);
    anyhow::ensure!(
        total_bytes == memory_bytes(total_mem, "-m 总内存")?,
        "/memory 大小 ({total_bytes:#x}) 与 -m ({total_mem}) 不一致"
    );
    let pmem_base = base + total_bytes - pmem_bytes;

    // 根节点 pmem-region 平台设备（of_pmem 绑定）
    let node = format!("/pmem@{pmem_base:x}");
    fdtput(&dtb, &["-c".to_string(), node.clone()])?;
    fdtput(
        &dtb,
        &[
            "-t".to_string(),
            "s".to_string(),
            node.clone(),
            "compatible".to_string(),
            "pmem-region".to_string(),
        ],
    )?;
    let mut prop_args = vec![
        "-t".to_string(),
        "x".to_string(),
        node.clone(),
        "reg".to_string(),
    ];
    prop_args.extend(dt_cells(pmem_base, addr_cells));
    prop_args.extend(dt_cells(pmem_bytes, size_cells));
    fdtput(&dtb, &prop_args)?;
    Ok(dtb)
}

/// firecracker 后端不支持 virtio-serial 通道 —— agent 组件启用时提示被忽略。
pub(crate) fn warn_agent_unsupported(cfg: &Config) {
    if cfg.agent_enabled() {
        eprintln!(
            "WARN: [components.agent] enabled 但 firecracker 后端不支持 virtio-serial 通道 —— 组件被忽略"
        );
    }
}

/// firecracker 后端不支持 nvdimm —— pmem 组件启用时提示被忽略。
pub(crate) fn warn_pmem_unsupported(cfg: &Config) {
    if cfg.pmem_size().is_some() {
        eprintln!(
            "WARN: [components.pmem] enabled 但 firecracker 后端不支持 nvdimm —— 组件被忽略"
        );
    }
}
