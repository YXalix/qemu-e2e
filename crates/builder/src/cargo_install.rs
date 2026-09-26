//! cargo 构建装载共用流水线：`cargo build --release --target <musl triple>`
//! → 过滤二进制 → 拷贝 + 可执行位。testcases（`/tests/`）与 tools
//! （tools.img `/bin/`）同走此核心，差异只在目录、标签与降级文案。
//!
//! 降级规则（显式 WARN，不是吞错；构建失败仍 bail）：
//! - cargo 缺失或 musl target 未随 toolchain 安装 → WARN 跳过（返回 0）；
//! - 构建成功但无二进制 → bail。

use std::path::Path;

use anyhow::Context;

use crate::cross::CrossSetup;
use crate::Progress;

/// 一次装载任务的静态描述。
pub struct CargoInstall<'a> {
    /// 独立 cargo workspace 根（含 Cargo.toml）。
    pub rust_dir: &'a Path,
    /// 产物拷贝目的地（自动创建）。
    pub dest: &'a Path,
    pub triple: &'static str,
    pub cross: &'a CrossSetup,
    /// 日志/警告中的主题标签（"tools" / "rust testcases"）。
    pub label: &'a str,
    /// 逐二进制进度行前缀（"  Tool:" / "  Test:"）。
    pub item_prefix: &'a str,
}

impl CargoInstall<'_> {
    /// 执行构建与装载，返回装入的二进制数。
    pub fn run(&self, progress: &mut Progress) -> anyhow::Result<usize> {
        if !self.rust_dir.join("Cargo.toml").is_file() {
            return Ok(0);
        }
        if !common::fsutil::which("cargo") {
            progress.line(&format!(
                "  WARNING: {} skipped (cargo not found)",
                self.label
            ));
            return Ok(0);
        }
        if !crate::tools::musl_target_installed(self.triple) {
            progress.line(&format!(
                "  WARNING: {} skipped (rust target {} not installed; \
                 run `rustup target add {}` to enable)",
                self.label,
                self.triple,
                self.triple
            ));
            return Ok(0);
        }

        progress.line(&format!(
            "Building {} (cargo: musl static {})...",
            self.label, self.triple
        ));
        std::fs::create_dir_all(self.dest)?;
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["build", "--release", "--target", self.triple])
            .current_dir(self.rust_dir);
        self.cross.apply_to_cargo(&mut cmd);
        let out = cmd.output().context("cargo 启动失败")?;
        if !out.status.success() {
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            anyhow::bail!("{} build failed\n{log}", capitalize(self.label));
        }
        for line in String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.contains("warning:"))
        {
            progress.line(line);
        }

        let release_dir = self.rust_dir.join("target").join(self.triple).join("release");
        let mut installed = 0usize;
        for entry in std::fs::read_dir(&release_dir)?.flatten() {
            if let Some(bin) = crate::testcase::rust_test_binary(&entry.path()) {
                let name = bin
                    .file_name()
                    .context("release 目录条目缺文件名")?
                    .to_string_lossy()
                    .to_string();
                std::fs::copy(&bin, self.dest.join(&name))
                    .with_context(|| format!("拷贝 {} 失败", bin.display()))?;
                common::fsutil::set_executable(&self.dest.join(&name))?;
                progress.line(&format!("{} {name}", self.item_prefix));
                installed += 1;
            }
        }
        if installed == 0 {
            anyhow::bail!(
                "{} build succeeded but no binaries found under {}",
                capitalize(self.label),
                release_dir.display()
            );
        }
        Ok(installed)
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
