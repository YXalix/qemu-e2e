//! pmem 持久内存供给（[components.pmem]）：DT 途径（arm64/riscv64 直启无
//! ACPI，QEMU nvdimm 的 NFIT 需 EFI 引导才可见，故用 pmem-region 设备树节点
//! 绕开）。产线三件套落 target/build/pmem/：
//! 1) ram.img —— 主内存后端文件（memory-backend-file share=on），挖出的
//!    pmem 区即宿主文件 backed，guest 写入持久落盘；
//! 2) <machine>.dtb —— dumpdtb（按当前 -smp/-m 生成）+ fdtput 补丁（根节点
//!    注入 pmem-region@<base>），经 -dtb 传入替换 QEMU 生成版；
//! 3) mem_limit —— cmdline 追加 `mem=<总内存 − pmem 区>`：QEMU 会按 -m
//!    全量重建 /memory（无法缩 DTB），故由内核侧把 pmem 区排除出线性
//!    内存模型（等价 x86 memmap= 语义），devm_memremap_pages 才能建
//!    ZONE_DEVICE struct page（否则 sparse subsection 已存在 → -EEXIST）。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use launcher::{Arch, NumaTopology};

use crate::config::Config;

/// [components.pmem] enabled 才 Some（产线三件套见模块文档）。
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
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    let ram_backend = dir.join("ram.img");
    ensure_sparse(&ram_backend, total_bytes)?;
    let dtb = patch_pmem_dtb(cfg, arch, topo, &total_mem, &dir, pmem_bytes)?;
    Ok(Some(launcher::PmemSpec::new(
        size,
        mem_limit,
        ram_backend,
        dtb,
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
    cmd.arg("-machine")
        .arg(format!("{machine},dumpdtb={}", dtb.display()));
    cmd.args([
        "-smp",
        &topo.smp.to_string(),
        "-m",
        total_mem,
        "-display",
        "none",
    ]);
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
        Ok(fdtget_words(&dtb, "/", prop)?.first().copied().unwrap_or(2))
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
