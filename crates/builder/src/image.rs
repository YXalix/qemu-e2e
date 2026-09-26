//! 镜像打包（build-initrd.sh 尾段 + cpio2ext4.sh 的 Rust 接管）。
//! initramfs：原生 newc cpio + gzip（`builder::cpio`，无 GNU 工具依赖）；
//! rootfs：`du -sm + 2` 自动定容的 ext4（mke2fs -d，见 `find_mke2fs`）。

use std::path::{Path, PathBuf};

use anyhow::Context;

/// 定位 mke2fs：PATH → Homebrew e2fsprogs keg 路径（keg-only 不进 PATH，
/// Apple Silicon = /opt/homebrew，Intel = /usr/local）。
pub fn find_mke2fs() -> Option<PathBuf> {
    if let Some(p) = common::fsutil::which_path("mke2fs") {
        return Some(p);
    }
    [
        "/opt/homebrew/opt/e2fsprogs/sbin",
        "/usr/local/opt/e2fsprogs/sbin",
    ]
    .iter()
    .map(|d| Path::new(d).join("mke2fs"))
    .find(|p| p.is_file())
}

/// `mke2fs -q -F -t ext4 -L <label> -d <dir> <img> <size>M`，size = du -sm + 2。
pub fn make_ext4(dir: &Path, out: &Path, label: &str) -> anyhow::Result<()> {
    let du = std::process::Command::new("du")
        .arg("-sm")
        .arg(dir)
        .output()
        .context("failed to run du")?;
    if !du.status.success() {
        anyhow::bail!("du -sm {} failed", dir.display());
    }
    let mb: u64 = String::from_utf8_lossy(&du.stdout)
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .context("failed to parse du output")?;
    let size_mb = mb + 2;

    let mke2fs = find_mke2fs().ok_or_else(|| {
        anyhow::anyhow!(
            "mke2fs not found (Linux: e2fsprogs package; macOS: brew install e2fsprogs)"
        )
    })?;
    let _ = std::fs::remove_file(out);
    let status = std::process::Command::new(&mke2fs)
        .args(["-q", "-F", "-t", "ext4", "-L", label, "-d"])
        .arg(dir)
        .arg(out)
        .arg(format!("{size_mb}M"))
        .status()
        .with_context(|| format!("failed to run {} (install e2fsprogs)", mke2fs.display()))?;
    if !status.success() {
        anyhow::bail!("{label} ext4 build failed (mke2fs exit {status})");
    }
    Ok(())
}
