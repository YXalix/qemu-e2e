//! BusyBox 供给（fetch-busybox.sh 的 Rust 接管）。
//! 四层供给链（按序尝试，命中即返回）：
//!   0. 本地缓存 target/build/busybox/bin/busybox-<arch>
//!   1. BUSYBOX_DL_URL   显式完整资产 URL（wget/curl）
//!   2. gh release download（自带认证，私有仓库可用）
//!   3. 直链 wget/curl（公开 release）
//!   4. 源码编译兜底（busybox.net → GitHub mirror 归档；仅 Linux 宿主）
//!
//! 所有下载做 ELF 魔数校验（\x7fELF）。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use common::Arch;

use crate::Progress;

pub const DEFAULT_VERSION: &str = "1.36.1";

/// 供给配置（BUSYBOX_* 环境变量优先，回落 virtuoso.toml [busybox] 段）。
#[derive(Debug, Clone, Default)]
pub struct Supply {
    pub version: Option<String>,
    pub release_repo: Option<String>,
    pub dl_url: Option<String>,
    pub force_source_build: bool,
}

fn cache_bin(build_dir: &Path, arch: Arch) -> PathBuf {
    build_dir
        .join("busybox/bin")
        .join(format!("busybox-{}", arch.name()))
}

/// 确保目标架构静态 BusyBox 就位，返回其二进制路径。失败即 Err（构建中止）。
pub fn ensure(
    build_dir: &Path,
    arch: Arch,
    supply: &Supply,
    progress: &mut Progress,
) -> anyhow::Result<PathBuf> {
    let version = supply
        .version
        .clone()
        .unwrap_or_else(|| DEFAULT_VERSION.into());
    let bin = cache_bin(build_dir, arch);
    let asset = format!("busybox-{version}-linux-{}", arch.name());
    let tag = format!("busybox-v{version}");

    if bin.is_file() {
        progress.line(&format!("BusyBox: cached ({})", arch.name()));
        return Ok(bin);
    }
    std::fs::create_dir_all(build_dir.join("busybox/bin"))?;

    if !supply.force_source_build {
        // 1) 显式 URL
        if let Some(url) = &supply.dl_url {
            if try_wget(url, &bin, &asset, progress) {
                return Ok(bin);
            }
        }
        // 2/3) release 下载
        if let Some(repo) = resolve_repo(build_dir, &supply.release_repo) {
            // 2) gh（认证可用时）
            if common::fsutil::which("gh") && gh_auth_ok() {
                progress.line(&format!("Fetching {asset} via gh from {repo} ({tag})"));
                let tmp = bin.with_extension("tmp");
                if gh_download_asset(&repo, &tag, &asset, &tmp) && common::fsutil::is_elf(&tmp) {
                    std::fs::rename(&tmp, &bin)?;
                    common::fsutil::set_executable(&bin)?;
                    progress.line(&format!("BusyBox: downloaded via gh ({})", arch.name()));
                    return Ok(bin);
                }
                let _ = std::fs::remove_file(&tmp);
                progress.line("WARNING: gh release download failed, trying plain wget");
            }
            // 3) 直链
            let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
            if try_wget(&url, &bin, &asset, progress) {
                return Ok(bin);
            }
            progress.line("WARNING: release download failed, falling back to source build");
        } else {
            progress.line("WARNING: no release source available.");
            progress.line("  Set BUSYBOX_RELEASE_REPO=<owner>/<repo> (env or virtuoso.toml [busybox] release_repo; or push to GitHub with the busybox-release workflow).");
        }
    }

    // 4) 源码编译兜底
    build_from_source(build_dir, arch, &version, progress)?;
    Ok(bin)
}

fn try_wget(url: &str, bin: &Path, asset: &str, progress: &mut Progress) -> bool {
    let tmp = bin.with_extension("tmp");
    progress.line(&format!("Fetching {asset} from {url}"));
    let ok = fetch(url, &tmp) && common::fsutil::is_elf(&tmp);
    if ok {
        common::fsutil::set_executable(&tmp).ok();
        let _ = std::fs::rename(&tmp, bin);
        progress.line("BusyBox: downloaded");
        true
    } else {
        let _ = std::fs::remove_file(&tmp);
        false
    }
}

/// gh release 单资产下载到 `dest`（gh 在 PATH 且认证可用时才调用；
/// 成功 = exit 0）。busybox 与 preset 的 release 供给共用。
pub(crate) fn gh_download_asset(repo: &str, tag: &str, asset: &str, dest: &Path) -> bool {
    Command::new("gh")
        .args(["release", "download", tag, "-R", repo, "-p", asset, "-O"])
        .arg(dest)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 下载到 `tmp`：wget 优先，缺失回落 curl（macOS 无 wget 但自带 curl）。
pub(crate) fn fetch(url: &str, tmp: &Path) -> bool {
    let (bin, args) = if common::fsutil::which("wget") {
        ("wget", vec!["-q".to_string(), "-O".to_string()])
    } else {
        ("curl", vec!["-fsSL".to_string(), "-o".to_string()])
    };
    let mut c = Command::new(bin);
    c.args(&args).arg(tmp).arg(url);
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// applet 名单（busybox 树的符号链接生成用）三级解析：
/// 1. `BUSYBOX_APPLETS_FILE` 显式文件（自定义 busybox 配置时提供，
///    内容 = `busybox --list` 输出，每行一个 applet，# 注释）；
/// 2. `infra/busybox/applets-<version>.txt` —— 与 release 配方（defconfig +
///    CONFIG_STATIC=y、去 CONFIG_TC）一起冻结进仓库的版本名单，三架构同配置
///    共用一份；
/// 3. 宿主可直接执行该二进制（Linux 同构）→ `--list` 现取（兜底）。
///
/// 取代旧 `busybox --install` 宿主执行路径：macOS 无法 exec guest Linux ELF，
/// 名单驱动让符号链接生成与宿主架构/OS 解耦（冻结不变量「测试禁止掩盖失败」
/// 不受影响——名单缺失是显式报错）。
pub fn applet_names(infra_dir: &Path, version: &str, bin: &Path) -> anyhow::Result<Vec<String>> {
    if let Some(f) = std::env::var_os("BUSYBOX_APPLETS_FILE") {
        return read_applet_file(Path::new(&f));
    }
    let frozen = infra_dir
        .join("busybox")
        .join(format!("applets-{version}.txt"));
    if frozen.is_file() {
        return read_applet_file(&frozen);
    }
    if let Ok(out) = Command::new(bin).arg("--list").output() {
        if out.status.success() {
            let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect();
            if !names.is_empty() {
                return Ok(names);
            }
        }
    }
    anyhow::bail!(
        "无法确定 BusyBox applet 名单：infra/busybox/applets-{version}.txt 缺失，\
         且宿主无法执行该二进制（交叉组装）\n  \
         提供 BUSYBOX_APPLETS_FILE=<file>（`busybox --list` 输出）或补交名单文件"
    )
}

fn read_applet_file(p: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(p).with_context(|| format!("读取 {} 失败", p.display()))?;
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    anyhow::ensure!(!names.is_empty(), "{} 内容为空", p.display());
    Ok(names)
}

/// 推导发布仓库：显式 BUSYBOX_RELEASE_REPO → 从 start 逐级向上找含 `.git`
/// 的项目根，扫其 git remote 取 GitHub 的那个。
/// release 仓库名（owner/repo）：显式指定优先，否则解析项目 git origin 的
/// GitHub URL（busybox 与 kernel preset 两套供给共用）。
pub(crate) fn resolve_repo(start: &Path, explicit: &Option<String>) -> Option<String> {
    if let Some(repo) = explicit {
        return Some(repo.clone());
    }
    let mut project_root = start.to_path_buf();
    loop {
        if project_root.join(".git").exists() {
            break;
        }
        if !project_root.pop() {
            return None;
        }
    }
    let out = Command::new("git")
        .args(["-C", &project_root.display().to_string(), "remote", "-v"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let _name = it.next()?;
        let url = it.next()?;
        if line.ends_with("(fetch)") && url.contains("github.com") {
            let stripped = url
                .split_once("github.com")
                .map(|(_, rest)| rest.trim_start_matches([':', '/']))
                .unwrap_or(url);
            return Some(stripped.trim_end_matches(".git").to_string());
        }
    }
    None
}

pub(crate) fn gh_auth_ok() -> bool {
    Command::new("gh")
        .arg("auth")
        .arg("status")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_from_source(
    build_dir: &Path,
    arch: Arch,
    version: &str,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    if common::HostOs::current() != common::HostOs::Linux {
        anyhow::bail!(
            "BusyBox 源码兜底构建仅支持 Linux 宿主（产出需为 guest 架构 Linux ELF）。\n  \
             macOS：配置 BUSYBOX_RELEASE_REPO / BUSYBOX_DL_URL 走 release 下载层"
        );
    }
    let host_norm = Arch::parse(std::env::consts::ARCH)
        .map(|a| a.name())
        .unwrap_or("unknown");
    progress.line(&format!(
        "Building BusyBox {version} from source (host arch: {})...",
        std::env::consts::ARCH
    ));
    let busybox_root = build_dir.join("busybox");
    std::fs::create_dir_all(&busybox_root)?;
    let src_dir = busybox_root.join("busybox");

    if !src_dir.is_dir() {
        let tag = version.replace('.', "_"); // busybox tag: 1.36.1 → 1_36_1
        let tmp = busybox_root.join("src.tar");
        let tried = try_archive(
            &format!("https://busybox.net/downloads/busybox-{version}.tar.bz2"),
            "-xjf",
            &format!("busybox-{version}"),
            &tmp,
            &busybox_root,
            &src_dir,
        ) || try_archive(
            &format!("https://github.com/mirror/busybox/archive/refs/tags/{tag}.tar.gz"),
            "-xzf",
            &format!("busybox-{tag}"),
            &tmp,
            &busybox_root,
            &src_dir,
        );
        if !tried {
            anyhow::bail!("all BusyBox source archives failed");
        }
    }

    let run = |mut c: Command| -> anyhow::Result<()> {
        let status = c.status().with_context(|| "busybox 构建步骤失败")?;
        if !status.success() {
            anyhow::bail!("busybox 构建步骤退出码 {status}");
        }
        Ok(())
    };
    run(mk("make", &["defconfig"], &src_dir))?;
    // sed 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' —— Rust 等价实现
    let config_path = src_dir.join(".config");
    let config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        &config_path,
        config.replace("# CONFIG_STATIC is not set", "CONFIG_STATIC=y"),
    )?;
    let mut make = mk("make", &[], &src_dir);
    if let Ok(n) = std::thread::available_parallelism() {
        make.arg(format!("-j{n}"));
    }
    run(make)?;

    let bin = cache_bin(build_dir, arch);
    std::fs::copy(src_dir.join("busybox"), &bin)?;
    common::fsutil::set_executable(&bin)?;

    if host_norm != arch.name() {
        progress.line(&format!(
            "WARNING: source-built BusyBox is {host_norm} but target ARCH={}.\n  Cross-arch initramfs needs a prebuilt release binary:\n  run the busybox-release workflow, then set BUSYBOX_RELEASE_REPO (env or toml [busybox] release_repo).",
            arch.name()
        ));
    }
    progress.line(&format!(
        "BusyBox: built from source ({host_norm} binary cached as busybox-{})",
        arch.name()
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn try_archive(
    url: &str,
    tar_flags: &str,
    sub: &str,
    tmp: &Path,
    busybox_root: &Path,
    dst: &Path,
) -> bool {
    if fetch(url, tmp) {
        let untar = Command::new("tar")
            .arg(tar_flags)
            .arg(tmp)
            .arg("-C")
            .arg(busybox_root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(tmp);
        if untar {
            return std::fs::rename(busybox_root.join(sub), dst).is_ok();
        }
    } else {
        let _ = std::fs::remove_file(tmp);
    }
    false
}

fn mk(bin: &str, args: &[&str], cwd: &Path) -> Command {
    let mut c = Command::new(bin);
    c.args(args).current_dir(cwd);
    c
}
