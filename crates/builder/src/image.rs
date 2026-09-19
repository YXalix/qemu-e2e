//! 镜像打包（build-initrd.sh 尾段 + cpio2ext4.sh 的 Rust 接管）。
//! initramfs：newc cpio + gzip -9；rootfs：`du -sm + 2` 自动定容的 ext4。

use std::path::Path;

use anyhow::Context;

/// `(cd dir && find . -print0 | cpio --null -o -H newc 2>/dev/null) | gzip -9 > out`
/// （保留 null 分隔与脚本完全一致；文件顺序随 find，无需稳定排序）。
pub fn pack_initramfs(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let out_file =
        std::fs::File::create(out).with_context(|| format!("创建 {} 失败", out.display()))?;
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg("find . -print0 | cpio --null -o -H newc 2>/dev/null | gzip -9")
        .current_dir(dir)
        .stdout(out_file)
        .status()
        .context("cpio/gzip 启动失败（安装 cpio、gzip）")?;
    if !status.success() {
        anyhow::bail!("initramfs 打包失败（cpio 退出码 {status}）");
    }
    Ok(())
}

/// `mke2fs -q -F -t ext4 -L rootfs -d <dir> <img> <size>M`，size = du -sm + 2。
pub fn make_ext4(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let du = std::process::Command::new("du")
        .arg("-sm")
        .arg(dir)
        .output()
        .context("du 启动失败")?;
    if !du.status.success() {
        anyhow::bail!("du -sm {} 失败", dir.display());
    }
    let mb: u64 = String::from_utf8_lossy(&du.stdout)
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .context("du 输出解析失败")?;
    let size_mb = mb + 2;

    let _ = std::fs::remove_file(out);
    let status = std::process::Command::new("mke2fs")
        .args(["-q", "-F", "-t", "ext4", "-L", "rootfs", "-d"])
        .arg(dir)
        .arg(out)
        .arg(format!("{size_mb}M"))
        .status()
        .context("mke2fs 启动失败（安装 e2fsprogs）")?;
    if !status.success() {
        anyhow::bail!("rootfs ext4 构建失败（mke2fs 退出码 {status}）");
    }
    Ok(())
}
