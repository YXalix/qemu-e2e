//! 前置检查（verify.sh 的 Rust 接管）。输出文本与退出码语义对齐 shell 基线：
//! critical FAIL → exit 1；WARN/INFO 不影响退出码。

use std::path::Path;

use common::{Arch, HostOs};

use crate::modconf;

/// 宿主工具表（按平台）。initrd 打包已原生化（builder::cpio），cpio/gzip/
/// wget/nproc/timeout 不再是硬需求；下载层 wget 缺失时有 curl 回退。
pub fn host_tools(host: HostOs) -> &'static [&'static str] {
    match host {
        HostOs::Linux => &["tar", "make", "cmake", "find", "sed"],
        // sed 仅 busybox 源码兜底路径使用（非 Linux 宿主该路径直接拒绝）
        HostOs::Darwin => &["tar", "make", "cmake", "find"],
    }
}

/// 下载工具（busybox 供给层）：wget 或 curl 任一（macOS 自带 curl）。
fn fetch_tool_ok() -> bool {
    common::fsutil::which("wget") || common::fsutil::which("curl")
}

/// 组件计划条目（conf 行）的模块是否都能在内核树找到（检查 #7 的输入收集）。
/// `kernel_path` 为 None（未配置内核树）时全部记为未找到。
pub fn module_presence(
    module_lines: &[String],
    kernel_path: Option<&Path>,
    fallback_dir: &Path,
) -> Vec<(String, bool)> {
    module_lines
        .iter()
        .map(|line| modconf::module_name(line))
        .filter(|m| !m.is_empty())
        .map(|m| {
            let found = kernel_path
                .map(|kp| modconf::find_ko(kp, fallback_dir, m).is_some())
                .unwrap_or(false);
            (m.to_string(), found)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Pass,
    Fail,
    Warn,
    Info,
}

#[derive(Debug)]
pub struct Check {
    pub level: Level,
    pub msg: String,
}

fn pass(msg: impl Into<String>) -> Check {
    Check {
        level: Level::Pass,
        msg: msg.into(),
    }
}
fn fail(msg: impl Into<String>) -> Check {
    Check {
        level: Level::Fail,
        msg: msg.into(),
    }
}
fn warn(msg: impl Into<String>) -> Check {
    Check {
        level: Level::Warn,
        msg: msg.into(),
    }
}
fn info(msg: impl Into<String>) -> Check {
    Check {
        level: Level::Info,
        msg: msg.into(),
    }
}

pub struct Report {
    pub checks: Vec<Check>,
    pub critical_pass: u32,
    pub critical_fail: u32,
    pub warnings: u32,
}

/// 运行全部检查（verify.sh 的 11 项 + 组件平台门）。
#[allow(clippy::too_many_arguments)]
pub fn run_checks(
    config_file_exists: bool,
    kernel_path: Option<&Path>,
    arch: Arch,
    host: HostOs,
    host_is_cross: bool,
    kernel_image: Option<&Path>,
    qemu_bin: Option<&str>,
    qemu_override: Option<&str>,
    modules: &[(String, bool)], // (模块名, .ko 是否找到)
    busybox_cached: bool,
    tools_img_exists: bool,
    initrd: Option<&Path>,
    vfio_enabled: bool,
    pmem_enabled: bool,
) -> Report {
    let mut checks = Vec::new();

    // 1. Configuration
    if config_file_exists {
        checks.push(pass("Configuration: virtuoso.toml found"));
    } else {
        checks.push(fail(
            "Configuration: virtuoso.toml not found (run `virtuoso` inside the project root, or restore the shipped template)",
        ));
    }

    // 2. Host tools（按宿主平台分表 + 下载/镜像/交叉工具链）
    let tools = host_tools(host);
    let mut missing: Vec<String> = tools
        .iter()
        .filter(|t| !common::fsutil::which(t))
        .map(|t| (*t).to_string())
        .collect();
    if !fetch_tool_ok() {
        missing.push("wget|curl".into());
    }
    if crate::image::find_mke2fs().is_none() {
        missing.push("mke2fs".into());
    }
    // C 用例编译器：Linux = 宿主 gcc/cc；macOS = CC env 或 zig（交叉）
    let cc_ok = if host == HostOs::Darwin {
        std::env::var_os("CC").is_some() || common::fsutil::which("zig")
    } else {
        ["gcc", "cc"].iter().any(|c| common::fsutil::which(c))
    };
    if !cc_ok {
        missing.push(if host == HostOs::Darwin {
            "zig (brew install zig) or CC".into()
        } else {
            "gcc/cc".into()
        });
    }
    if missing.is_empty() {
        checks.push(pass(format!(
            "Host tools: all found ({}) [{}]",
            tools.join(" "),
            host.name()
        )));
    } else {
        let hint = if missing.iter().any(|m| m.starts_with("mke2fs")) && host == HostOs::Darwin {
            " (brew install e2fsprogs; keg-only 路径已自动探测)"
        } else {
            ""
        };
        checks.push(fail(format!(
            "Host tools: missing -{}{hint}",
            missing.join(" ")
        )));
    }

    // 3. KERNEL_PATH
    let kernel_path = match kernel_path.filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => {
            checks.push(pass(format!("KERNEL_PATH: {}", p.display())));
            Some(p.to_path_buf())
        }
        None => {
            checks.push(fail(
                "KERNEL_PATH not set (set kernel_path in virtuoso.toml or KERNEL_PATH env var)",
            ));
            None
        }
    };

    // 4. Kernel source tree (+ version)
    let kernel_ok = kernel_path
        .as_ref()
        .map(|p| p.join("arch").is_dir())
        .unwrap_or(false);
    match (&kernel_path, kernel_ok) {
        (Some(p), true) => {
            let kver = kernel_version(p);
            checks.push(pass(format!(
                "Kernel source: {}{}",
                p.display(),
                kver.map(|v| format!(" (v{v})")).unwrap_or_default()
            )));
        }
        (Some(p), false) => checks.push(fail(format!(
            "Kernel source: {}/arch not found (set KERNEL_PATH)",
            p.display()
        ))),
        (None, _) => {}
    }

    // 5. Kernel image
    match kernel_image.filter(|p| p.is_file()) {
        Some(img) => {
            let size = std::fs::metadata(img).map(|m| m.len()).unwrap_or(0);
            checks.push(pass(format!(
                "Kernel image: {} ({})",
                arch.kernel_img(),
                common::fmt::human_size_ls(size)
            )));
        }
        None => {
            checks.push(fail(format!(
                "Kernel image: {} not found (build the kernel first)",
                arch.kernel_img()
            )));
        }
    }

    // 6. QEMU binary
    let qemu_found = qemu_bin.is_some_and(common::fsutil::which);
    let qemu_override = qemu_override.filter(|s| !s.is_empty());
    if let Some(q) = qemu_override {
        if qemu_found {
            checks.push(pass(format!("QEMU binary: {q} (from env)")));
        } else {
            checks.push(fail(format!(
                "QEMU binary: {q} not found (from QEMU env var)"
            )));
        }
    } else if qemu_found {
        checks.push(pass(format!("QEMU binary: {}", arch.qemu_bin())));
    } else {
        checks.push(fail(format!(
            "QEMU binary: {} not found (install qemu-system-{})",
            arch.qemu_bin(),
            arch.name()
        )));
    }
    if common::fsutil::which("qemu-img") {
        checks.push(info("qemu-img: available"));
    } else {
        checks.push(info(
            "qemu-img: not found (optional, for manual disk image work)",
        ));
    }

    // 7. Kernel modules（WARN，不判死；清单 = 启用组件 require 并集 + boot 基础集附加）
    if modules.is_empty() {
        checks.push(info(
            "Kernel modules: none required (no enabled component declares require)",
        ));
    } else {
        let total = modules.len();
        let found = modules.iter().filter(|(_, ok)| *ok).count();
        if found == total {
            checks.push(pass(format!("Kernel modules: {found}/{total} found")));
        } else {
            let missing: Vec<&str> = modules
                .iter()
                .filter(|(_, ok)| !ok)
                .map(|(m, _)| m.as_str())
                .collect();
            checks.push(warn(format!(
                "Kernel modules: {found}/{total} found, missing: {}",
                missing.join(" ")
            )));
        }
    }

    // 8. BusyBox
    if busybox_cached {
        checks.push(pass(format!("BusyBox: cached ({})", arch.name())));
    } else {
        checks.push(info(format!(
            "BusyBox: not cached for {} (release download on first build, source build fallback)",
            arch.name()
        )));
    }

    // 9. Cross-compile
    if host_is_cross {
        checks.push(warn(format!(
            "Cross-compile: ARCH={} differs from host ({}); C 用例走 zig/CC，Rust 走 rustup musl target",
            arch.name(),
            std::env::consts::ARCH
        )));
    }

    // 10. Tools image
    checks.push(if tools_img_exists {
        info("Tools image: exists (attached as /dev/vdb, mounted at /tools)")
    } else {
        info("Tools image: not built yet (run `virtuoso build`)")
    });

    // 11. initrd
    match initrd.filter(|p| p.is_file()) {
        Some(p) => {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            checks.push(info(format!(
                "Initrd: {} ({})",
                p.display(),
                common::fmt::human_size_ls(size)
            )));
        }
        None => checks.push(info("Initrd: not built yet (run 'make initrd')")),
    }

    // 12. 组件平台门：vfio 架构性依赖 Linux（IOMMU + vfio-pci）；pmem 的
    // memory-backend-file/dumpdtb 链路在 macOS 上未经实测（brew dtc 可得）
    if vfio_enabled {
        if host == HostOs::Darwin {
            checks.push(fail(
                "Components: vfio requires Linux host (IOMMU + vfio-pci) — 关闭 [components.vfio]",
            ));
        } else {
            checks.push(pass("Components: vfio enabled (Linux + IOMMU)"));
        }
    }
    if pmem_enabled && host == HostOs::Darwin {
        checks.push(warn(
            "Components: pmem on macOS is experimental (memory-backend-file + dumpdtb/fdtput 未在 HVF 实测)",
        ));
    }

    let critical_pass = checks.iter().filter(|c| c.level == Level::Pass).count() as u32;
    let critical_fail = checks.iter().filter(|c| c.level == Level::Fail).count() as u32;
    let warnings = checks.iter().filter(|c| c.level == Level::Warn).count() as u32;
    Report {
        checks,
        critical_pass,
        critical_fail,
        warnings,
    }
}

fn kernel_version(kernel: &Path) -> Option<String> {
    let makefile = std::fs::read_to_string(kernel.join("Makefile")).ok()?;
    let mut parts = Vec::new();
    for key in ["VERSION", "PATCHLEVEL", "SUBLEVEL"] {
        for line in makefile.lines() {
            if let Some(v) = line.strip_prefix(&format!("{key} = ")) {
                parts.push(v.trim().to_string());
                break;
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("."))
}

impl Report {
    /// 渲染（tty 下着色）。
    pub fn render(&self) -> String {
        let tty = is_stdout_tty();
        let c = |code: &str, s: &str| {
            if tty {
                format!("\x1b[{code}m{s}\x1b[0m")
            } else {
                s.to_string()
            }
        };
        let mut out = String::new();
        out.push_str(&c("1", "[VERIFY] QEMU E2E Prerequisites Check"));
        out.push_str("\n========================================\n");
        for chk in &self.checks {
            let (tag, code) = match chk.level {
                Level::Pass => ("[PASS]", "0;32"),
                Level::Fail => ("[FAIL]", "0;31"),
                Level::Warn => ("[WARN]", "0;33"),
                Level::Info => ("[INFO]", "0;36"),
            };
            out.push_str(&format!("  {} {}\n", c(code, tag), chk.msg));
        }
        out
    }
}

/// 仅影响颜色，不影响判定（doctor 呈现层共用同一 NO_COLOR 语义）。
pub fn is_stdout_tty() -> bool {
    // 无 libc 依赖的近似判断：TERM 存在且非 dumb，且没有 CI 强制管道。
    // 仅影响颜色，不影响判定。
    std::env::var_os("TERM")
        .map(|t| t != "dumb")
        .unwrap_or(false)
        && std::env::var_os("NO_COLOR").is_none()
}
