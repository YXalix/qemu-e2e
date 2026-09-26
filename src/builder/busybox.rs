//! BusyBox 供给（fetch-busybox.sh 的 Rust 接管）。
//! 供给链 = GitHub 拉取 + 本地缓存复用（按序尝试，命中即缓存返回）：
//!   0. 本地缓存 target/build/busybox/bin/busybox-<version>-linux-<arch>
//!   1. BUSYBOX_DL_URL   显式完整资产 URL（wget/curl）
//!   2. gh release download（自带认证，私有仓库可用）
//!   3. 直链 wget/curl（公开 release）
//!
//! 无源码编译兜底：全部未命中即报错（静态二进制统一由
//! .github/workflows/busybox-release.yml 发布）。所有下载做 ELF 魔数校验。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use crate::Arch;

use crate::util::Progress;

pub(crate) const DEFAULT_VERSION: &str = "1.36.1";

/// 生效版本（BUSYBOX_VERSION / toml [busybox] version，缺省回 DEFAULT_VERSION）。
pub(crate) fn effective_version(supply: &Supply) -> &str {
    supply.version.as_deref().unwrap_or(DEFAULT_VERSION)
}

/// 供给配置（BUSYBOX_* 环境变量优先，回落 virtuoso.toml [busybox] 段）。
#[derive(Debug, Clone, Default)]
pub(crate) struct Supply {
    pub version: Option<String>,
    pub release_repo: Option<String>,
    pub dl_url: Option<String>,
}

/// 缓存二进制路径：文件名与 release 资产名同构（busybox-<version>-linux-<arch>），
/// 版本进缓存键——换版本即未命中，不会误用旧版二进制。
pub(crate) fn cache_bin(build_dir: &Path, version: &str, arch: Arch) -> PathBuf {
    build_dir
        .join("busybox/bin")
        .join(format!("busybox-{version}-linux-{}", arch.name()))
}

/// 确保目标架构静态 BusyBox 就位，返回其二进制路径。失败即 Err（构建中止）。
pub(crate) fn ensure(
    build_dir: &Path,
    arch: Arch,
    supply: &Supply,
    progress: &mut Progress,
) -> anyhow::Result<PathBuf> {
    let version = effective_version(supply);
    let bin = cache_bin(build_dir, version, arch);
    let asset = format!("busybox-{version}-linux-{}", arch.name());
    let tag = format!("busybox-v{version}");

    if bin.is_file() {
        progress.line(&format!("BusyBox: cached {version} ({})", arch.name()));
        return Ok(bin);
    }
    std::fs::create_dir_all(build_dir.join("busybox/bin"))?;

    // 1) 显式 URL
    if let Some(url) = &supply.dl_url {
        if try_wget(url, &bin, &asset, progress) {
            return Ok(bin);
        }
    }
    // 2/3) release 下载
    let repo = resolve_repo(build_dir, &supply.release_repo);
    if let Some(repo) = repo {
        // 2) gh（认证可用时）
        if crate::util::which("gh") && gh_auth_ok() {
            progress.line(&format!("Fetching {asset} via gh from {repo} ({tag})"));
            let tmp = bin.with_extension("tmp");
            if gh_download_asset(&repo, &tag, &asset, &tmp) && crate::util::is_elf(&tmp) {
                std::fs::rename(&tmp, &bin)?;
                crate::util::set_executable(&bin)?;
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
        anyhow::bail!(
            "BusyBox {asset} 下载失败（{repo} @ {tag}，gh 与直链均未命中）。\n  \
             本地无源码编译兜底：静态 busybox 统一由 GitHub Release 供给。\n  \
             - 检查网络/代理，或改用 BUSYBOX_DL_URL=<asset-url> 直连资产\n  \
             - 仓库自发布：推送 tag busybox-v{version} 触发 .github/workflows/busybox-release.yml"
        );
    }
    anyhow::bail!(
        "BusyBox {asset} 无供给来源（缓存未命中，也未配置 release 仓库）。\n  \
         本地无源码编译兜底：静态 busybox 统一由 GitHub Release 供给。\n  \
         - 配 BUSYBOX_RELEASE_REPO=<owner>/<repo>（env 或 toml [busybox] release_repo）\n  \
         - 或用 BUSYBOX_DL_URL=<asset-url> 直连资产\n  \
         - 仓库自发布：推送 tag busybox-v{version} 触发 .github/workflows/busybox-release.yml"
    );
}

fn try_wget(url: &str, bin: &Path, asset: &str, progress: &mut Progress) -> bool {
    let tmp = bin.with_extension("tmp");
    progress.line(&format!("Fetching {asset} from {url}"));
    let ok = fetch(url, &tmp) && crate::util::is_elf(&tmp);
    if ok {
        crate::util::set_executable(&tmp).ok();
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
    let (bin, args) = if crate::util::which("wget") {
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
pub(crate) fn applet_names(infra_dir: &Path, version: &str, bin: &Path) -> anyhow::Result<Vec<String>> {
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
    let text =
        std::fs::read_to_string(p).with_context(|| format!("read {} failed", p.display()))?;
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    anyhow::ensure!(!names.is_empty(), "{} is empty", p.display());
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
