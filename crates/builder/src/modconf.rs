//! modules.conf / modules-boot.conf 语义（build-initrd.sh 的 Rust 接管）。
//! 每行 `<module> [key=val ...]`；空白与 `#` 注释行跳过；首 token 是模块名，
//! 其余 token 由 VM 内 init 传给 insmod（复制阶段只关心 `.ko` 文件）。

use std::path::Path;

use anyhow::Context;

use crate::Progress;

/// 解析 conf：返回模块名列表（保持声明顺序 —— 依赖手工排序是冻结语义）。
pub fn parse(conf: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(conf) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// 在内核树中定位 `<mod>.ko`（首个命中，等价 `find -print -quit`）；
/// 兜底 `infra/testcases/<mod>.ko`。公开给 verify 的模块存在性检查复用。
pub fn find_ko(kernel_path: &Path, infra_dir: &Path, module: &str) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("find")
        .arg(kernel_path)
        .arg("-name")
        .arg(format!("{module}.ko"))
        .arg("-print")
        .arg("-quit")
        .output()
        .ok()?;
    let out_text = String::from_utf8_lossy(&out.stdout).to_string();
    let first = out_text.lines().next();
    if let Some(p) = first.filter(|s| !s.is_empty()) {
        return Some(std::path::PathBuf::from(p));
    }
    let fallback = infra_dir.join("testcases").join(format!("{module}.ko"));
    fallback.is_file().then_some(fallback)
}

/// 复制 conf 声明的全部模块到 dest，并把 conf 原样随行
/// （VM 内 init 从 /lib/modules/modules[-boot].conf 读取加载顺序与参数）。
/// 缺 `.ko` 构建期报错（退出码非 0，冻结语义）。
pub fn copy_modules(
    dest: &Path,
    conf: &Path,
    kernel_path: &Path,
    infra_dir: &Path,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    if !conf.is_file() {
        progress.line(&format!(
            "  WARNING: {} not found, skipping its modules",
            conf.file_name().unwrap_or_default().to_string_lossy()
        ));
        return Ok(());
    }
    progress.line(&format!(
        "Copying kernel modules from {}...",
        conf.file_name().unwrap_or_default().to_string_lossy()
    ));
    for module in parse(conf) {
        let Some(ko) = find_ko(kernel_path, infra_dir, &module) else {
            anyhow::bail!("Module {module}.ko not found");
        };
        std::fs::copy(&ko, dest.join(format!("{module}.ko")))
            .with_context(|| format!("复制 {} 失败", ko.display()))?;
    }
    std::fs::copy(conf, dest.join(conf.file_name().unwrap_or_default()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_comments_and_args() {
        let dir = std::env::temp_dir().join(format!("builder-modconf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("modules.conf");
        std::fs::write(
            &conf,
            "# comment\ncrc64\nnvme-core  poll_queues=2\n\n  # indented\next4\n",
        )
        .unwrap();
        let mods = parse(&conf);
        assert_eq!(mods, ["crc64", "nvme-core", "ext4"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
