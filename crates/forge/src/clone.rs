//! clone 流水线：建卷 → git clone 进卷 → 渲染 .clangd → 写 current。

use std::path::Path;

use anyhow::Context;
use common::Arch;

use crate::state::{self, Current};
use crate::toolchain::{self, shell_quote};
use crate::{volume, Progress};

/// .clangd 模板（原 devkit/docker/.clangd 迁入；--target 行由 render_clangd
/// 按目标架构替换）。
const CLANGD_TEMPLATE: &str = include_str!("kernel.clangd");

/// --target=<triple> 行替换（退役 kernel.sh 的 sed：`s/--target=[a-z0-9_-]*/…/`）。
pub fn render_clangd(target: &str) -> String {
    const NEEDLE: &str = "--target=";
    let mut out = String::new();
    let mut rest = CLANGD_TEMPLATE;
    while let Some(i) = rest.find(NEEDLE) {
        out.push_str(&rest[..i + NEEDLE.len()]);
        rest = &rest[i + NEEDLE.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        out.push_str(target);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// 完整 clone：镜像供给 → 幂等建卷（已有内容拒绝）→ git clone --depth 1 →
/// .clangd 按架构渲染进源码根 → 写 current。返回宿主可见路径。
#[allow(clippy::too_many_arguments)]
pub fn run(
    project_root: &Path,
    volume_name: &str,
    arch: Arch,
    url: &str,
    ref_name: &str,
    image: &str,
    dockerfile_dir: &Path,
    progress: &mut Progress,
) -> anyhow::Result<std::path::PathBuf> {
    toolchain::ensure_image(image, dockerfile_dir, progress)?;

    let out = std::process::Command::new("docker")
        .args(["volume", "create", volume_name])
        .output()
        .context("运行 docker volume create 失败（docker 在位？）")?;
    anyhow::ensure!(
        out.status.success(),
        "docker volume create {volume_name} 失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );

    let view = volume::host_view(volume_name)?;
    anyhow::ensure!(
        volume::status(&view) == volume::Status::Empty,
        "volume {volume_name} 已有内容（换 `--as <新卷名>`，或 `virtuoso kernel use` 切过去后走增量构建）"
    );

    progress.line(&format!(
        "Kernel: cloning {url}@{ref_name} → volume {volume_name} ..."
    ));
    let script = format!(
        "git clone --depth 1 --branch {} {} /tmp/k && cp -a /tmp/k/. /ksrc/",
        shell_quote(ref_name),
        shell_quote(url)
    );
    toolchain::run_streaming(volume_name, image, None, &[], &script)?;

    progress.line(&format!(
        "Kernel: rendering .clangd (--target={}) ...",
        toolchain::clangd_target(arch)
    ));
    toolchain::write_file(
        volume_name,
        image,
        "/ksrc/.clangd",
        &render_clangd(toolchain::clangd_target(arch)),
    )?;

    state::write(
        project_root,
        &Current {
            volume: volume_name.to_string(),
            arch: arch.name().to_string(),
        },
    )?;
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_clangd_swaps_target_triple() {
        let rendered = render_clangd("riscv64-linux-gnu");
        assert!(rendered.contains("--target=riscv64-linux-gnu"));
        assert!(!rendered.contains("aarch64-linux-gnu"));
        // 模板其余内容原样保留（Remove 列表的尾行紧跟 --target 行之后）
        assert!(rendered.contains("-fno-spell-checking"));
        assert!(rendered.contains("MissingIncludes: None"));
    }

    #[test]
    fn render_clangd_is_idempotent_for_default() {
        assert_eq!(render_clangd("aarch64-linux-gnu"), CLANGD_TEMPLATE);
    }
}
