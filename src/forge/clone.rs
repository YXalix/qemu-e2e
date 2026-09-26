//! clone 流水线：建卷 → git clone 进卷 → 渲染 .clangd → 写 current。

use std::path::Path;

use anyhow::Context;
use crate::Arch;

use crate::forge::state::{self, Current};
use crate::forge::toolchain::{self, shell_quote};
use crate::forge::volume;
use crate::progress::Progress;

/// .clangd 模板源（devkit/docker/.clangd，唯一事实来源；--target 行由
/// render_clangd 按目标架构替换）。
pub(crate) fn clangd_template(project_root: &Path) -> anyhow::Result<String> {
    let path = project_root.join("devkit").join("docker").join(".clangd");
    std::fs::read_to_string(&path).with_context(|| {
        format!(
            "read .clangd template failed (is {} in place?)",
            path.display()
        )
    })
}

/// --target=<triple> 行替换（退役 kernel.sh 的 sed：`s/--target=[a-z0-9_-]*/…/`）。
pub(crate) fn render_clangd(template: &str, target: &str) -> String {
    const NEEDLE: &str = "--target=";
    let mut out = String::new();
    let mut rest = template;
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

/// 完整 clone 任务的输入（一次装配，免长参数表）。
pub(crate) struct CloneJob<'a> {
    pub project_root: &'a Path,
    pub volume_name: &'a str,
    pub arch: Arch,
    pub url: &'a str,
    pub ref_name: &'a str,
    pub image: &'a str,
    pub dockerfile_dir: &'a Path,
}

/// 完整 clone：镜像供给 → 幂等建卷（已有内容拒绝）→ git clone --depth 1 →
/// .clangd 按架构渲染进源码根 → 写 current。返回宿主可见路径。
pub(crate) fn run(job: &CloneJob, progress: &mut Progress) -> anyhow::Result<std::path::PathBuf> {
    let CloneJob {
        project_root,
        volume_name,
        arch,
        url,
        ref_name,
        image,
        dockerfile_dir,
    } = *job;
    toolchain::ensure_image(image, dockerfile_dir, progress)?;

    let out = std::process::Command::new("docker")
        .args(["volume", "create", volume_name])
        .output()
        .context("failed to run docker volume create (is docker present?)")?;
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
        "git clone --progress --depth 1 --branch {} {} /tmp/k && cp -a /tmp/k/. /ksrc/",
        shell_quote(ref_name),
        shell_quote(url)
    );
    toolchain::run_streaming(volume_name, image, &[], &script)?;

    progress.line(&format!(
        "Kernel: rendering .clangd (--target={}) ...",
        toolchain::clangd_target(arch)
    ));
    let template = clangd_template(project_root)?;
    toolchain::write_file(
        volume_name,
        image,
        "/ksrc/.clangd",
        &render_clangd(&template, toolchain::clangd_target(arch)),
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
        let template =
            "CompileFlags:\n  Add:\n    - --target=aarch64-linux-gnu\n    - -fno-spell-checking\n";
        let rendered = render_clangd(template, "riscv64-linux-gnu");
        assert!(rendered.contains("--target=riscv64-linux-gnu"));
        assert!(!rendered.contains("aarch64-linux-gnu"));
        assert!(rendered.contains("-fno-spell-checking"));
    }

    #[test]
    fn render_clangd_is_idempotent_for_default() {
        let template = "Add:\n    - --target=aarch64-linux-gnu\n    - -fno-spell-checking\n";
        assert_eq!(render_clangd(template, "aarch64-linux-gnu"), template);
    }
}
