//! preset 内核供给 —— mainline 预编 mini 内核（GitHub Release 资产）的
//! 版本解析与下载缓存。
//!
//! 定位：mini 内核只承担「装置 E2E 与开箱即用」（harness 引导链路：init →
//! busybox → testcases → judge），不是被测内核行为的权威——后者仍是
//! KERNEL_PATH 源码树（openEuler）。发布由 kernel-release.yml 承担：每次
//! mainline release 以 defconfig + `infra/kernel/fragment.config` 构建三
//! 架构（全 =y 零模块），发 GitHub Release：
//!
//!   tag    kernel-v<版本>        （如 kernel-v6.12.8）
//!   资产   Image-<arch> · config-<arch> · SHA256SUMS
//!
//! 版本解析顺序：调用方显式指定 > `infra/kernel/pin` 钉定 > 最新已发布。
//! 下载校验：SHA256SUMS 逐资产核对（缺失时退回大小下限防截断）。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use common::Arch;

use crate::{busybox, Progress};

pub const TAG_PREFIX: &str = "kernel-v";

/// 缓存里的镜像文件名（与 release 资产同名）。
pub fn image_name(arch: Arch) -> String {
    format!("Image-{}", arch.name())
}

/// 规范化版本号：容忍 "v" 前缀，校验 `X.Y[.Z]` 形态（mainline tag 语义）。
pub fn normalize_version(s: &str) -> Option<String> {
    let v = s.trim().trim_start_matches('v');
    let ok = !v.is_empty()
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && (1..=3).contains(&v.split('.').count());
    ok.then(|| v.to_string())
}

/// tag ↔ 版本互转（`6.12.8` ↔ `kernel-v6.12.8`）。
pub fn tag_for(version: &str) -> String {
    format!("{TAG_PREFIX}{version}")
}

fn version_of_tag(tag: &str) -> Option<String> {
    tag.strip_prefix(TAG_PREFIX).and_then(normalize_version)
}

/// `infra/kernel/pin` 钉定版本（无文件/全注释/空 = 未钉定 → None）。
pub fn pin_version(infra_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(infra_dir.join("kernel/pin")).ok()?;
    raw.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(normalize_version)
}

/// release 仓库名（owner/repo）：显式（env/toml 上游）优先，回落 git origin。
pub fn release_repo(explicit: Option<&str>, project_root: &Path) -> Option<String> {
    busybox::resolve_repo(project_root, &explicit.map(str::to_string))
}

/// 最新已发布版本：gh 认证可用走 API，否则跟随 releases/latest 的重定向。
pub fn latest_released(repo: &str) -> anyhow::Result<String> {
    if common::fsutil::which("gh") && busybox::gh_auth_ok() {
        let out = Command::new("gh")
            .args([
                "api",
                &format!("repos/{repo}/releases/latest"),
                "--jq",
                ".tag_name",
            ])
            .output()
            .context("failed to run gh api for the latest release")?;
        if out.status.success() {
            let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Some(v) = version_of_tag(&tag) {
                return Ok(v);
            }
        }
    }
    // 免认证路径：/releases/latest 302 → /releases/tag/kernel-v<ver>，
    // curl 的 %{url_effective} 给出最终 URL。
    if common::fsutil::which("curl") {
        let out = Command::new("curl")
            .args([
                "-sIL",
                "-o",
                "/dev/null",
                "-w",
                "%{url_effective}",
                &format!("https://github.com/{repo}/releases/latest"),
            ])
            .output()
            .context("failed to query the latest preset-kernel release (curl)")?;
        let url = String::from_utf8_lossy(&out.stdout);
        let tag = url.rsplit('/').next().unwrap_or_default();
        if let Some(v) = version_of_tag(tag) {
            return Ok(v);
        }
    }
    anyhow::bail!("no published preset kernel found (no kernel-v* release on {repo}) — run the kernel-release workflow first, or pass --version / pin infra/kernel/pin explicitly")
}

/// 确保单架构 preset 镜像就位（已缓存直接复用），返回其路径。
pub fn ensure_image(
    repo: &str,
    version: &str,
    dir: &Path,
    arch: Arch,
    progress: &mut Progress,
) -> anyhow::Result<PathBuf> {
    let img = dir.join(image_name(arch));
    if img.is_file() {
        progress.line(&format!("Kernel preset: cached ({})", arch.name()));
        return Ok(img);
    }
    std::fs::create_dir_all(dir).with_context(|| format!("create {} failed", dir.display()))?;
    let tag = tag_for(version);
    let asset = image_name(arch);
    let tmp = dir.join(format!("{asset}.tmp"));

    let mut downloaded = false;
    if common::fsutil::which("gh") && busybox::gh_auth_ok() {
        progress.line(&format!("Fetching {asset} via gh ({tag})"));
        downloaded = busybox::gh_download_asset(repo, &tag, &asset, &tmp);
    }
    if !downloaded {
        let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
        progress.line(&format!("Fetching {asset} from {url}"));
        downloaded = busybox::fetch(&url, &tmp);
    }
    anyhow::ensure!(
        downloaded,
        "{asset} 下载失败（{repo}@{tag}）——检查 release 是否存在/网络可达"
    );

    match verify_sha256(dir, repo, &tag, &asset, &tmp) {
        Ok(()) => {}
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    std::fs::rename(&tmp, &img).with_context(|| format!("place {asset} failed"))?;
    progress.line(&format!("Kernel preset: downloaded ({})", arch.name()));
    Ok(img)
}

/// SHA256SUMS 逐资产核对；SUMS 下载失败时退回大小下限（防截断，不防篡改
/// ——HTTPS + tag 已提供完整性，此处尽力而为）。
fn verify_sha256(
    dir: &Path,
    repo: &str,
    tag: &str,
    asset: &str,
    file: &Path,
) -> anyhow::Result<()> {
    let sums_tmp = dir.join("SHA256SUMS.tmp");
    let sums_url = format!("https://github.com/{repo}/releases/download/{tag}/SHA256SUMS");
    if !busybox::fetch(&sums_url, &sums_tmp) {
        let _ = std::fs::remove_file(&sums_tmp);
        let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
        anyhow::ensure!(
            size >= 4 * 1024 * 1024,
            "{asset} 大小异常（{size} 字节）且无 SHA256SUMS 可核对——疑似下载截断"
        );
        return Ok(());
    }
    let text = std::fs::read_to_string(&sums_tmp)?;
    let _ = std::fs::remove_file(&sums_tmp);
    let expect = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let hash = it.next()?;
            let name = it.next()?;
            (name == asset).then(|| hash.to_string())
        })
        .next()
        .with_context(|| format!("SHA256SUMS has no entry for {asset}"))?;
    let actual = sha256_hex(file).context(
        "no sha256sum/shasum on host, cannot verify SHA256SUMS (re-run to retry the download)",
    )?;
    anyhow::ensure!(
        actual == expect,
        "{asset} 校验和不匹配（期望 {expect}，实际 {actual}）——重新 fetch"
    );
    Ok(())
}

/// 宿主 sha256：`sha256sum`（Linux/coreutils）优先，macOS 回落 `shasum -a 256`。
fn sha256_hex(file: &Path) -> Option<String> {
    let tools: [(&str, &[&str]); 2] = [("sha256sum", &[]), ("shasum", &["-a", "256"])];
    for (bin, args) in tools {
        if !common::fsutil::which(bin) {
            continue;
        }
        if let Ok(out) = Command::new(bin).args(args).arg(file).output() {
            if let Some(h) = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
            {
                if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Some(h.to_ascii_lowercase());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_version_accepts_kernel_shapes() {
        assert_eq!(normalize_version("6.12.8").as_deref(), Some("6.12.8"));
        assert_eq!(normalize_version("v6.12").as_deref(), Some("6.12"));
        assert_eq!(normalize_version(" 6.13 ").as_deref(), Some("6.13"));
        assert_eq!(normalize_version("6.12.8.1"), None, "超过三段拒绝");
        assert_eq!(normalize_version("6.x"), None);
        assert_eq!(normalize_version(""), None);
        assert_eq!(normalize_version("6..8"), None);
    }

    #[test]
    fn tag_roundtrip() {
        assert_eq!(tag_for("6.12.8"), "kernel-v6.12.8");
        assert_eq!(version_of_tag("kernel-v6.12.8").as_deref(), Some("6.12.8"));
        assert_eq!(version_of_tag("v6.12.8"), None, "无前缀 tag 不认");
        assert_eq!(version_of_tag("busybox-v1.36.1"), None);
    }

    #[test]
    fn pin_reads_first_comment_free_line() {
        let dir = std::env::temp_dir().join(format!("vso-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("kernel")).unwrap();
        assert_eq!(pin_version(&dir), None, "无 pin 文件");

        std::fs::write(dir.join("kernel/pin"), "# 注释\n\n6.12.8\n").unwrap();
        assert_eq!(pin_version(&dir).as_deref(), Some("6.12.8"));

        std::fs::write(dir.join("kernel/pin"), "# 只有注释\n").unwrap();
        assert_eq!(pin_version(&dir), None, "空钉定 = 跟随最新");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_name_matches_asset_naming() {
        assert_eq!(image_name(Arch::Arm64), "Image-arm64");
        assert_eq!(image_name(Arch::X86_64), "Image-x86_64");
        assert_eq!(image_name(Arch::Riscv64), "Image-riscv64");
    }
}
