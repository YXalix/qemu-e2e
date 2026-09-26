//! cargo 构建装载共用流水线：`cargo build --release --target <musl triple>`
//! → 过滤二进制 → 拷贝 + 可执行位。testcases（`/tests/`）与 tools
//! （tools.img `/bin/`）同走此核心，差异只在目录、标签与降级文案。
//!
//! 降级规则（显式 WARN，不是吞错；构建失败仍 bail）：
//! - cargo 缺失或 musl target 未随 toolchain 安装 → WARN 跳过（返回 0）；
//! - 构建成功但无二进制 → bail。

use std::path::Path;

use anyhow::Context;

use crate::builder::cross::CrossSetup;
use crate::util::Progress;

/// 一次装载任务的静态描述。
pub(crate) struct CargoInstall<'a> {
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
    /// 二进制名 → 安装子目录（测试组）。None 或未命中 = 平铺进 dest（tools）。
    pub groups: Option<&'a std::collections::HashMap<String, String>>,
}

impl CargoInstall<'_> {
    /// 执行构建与装载，返回装入的二进制数。
    pub(crate) fn run(&self, progress: &mut Progress) -> anyhow::Result<usize> {
        if !self.rust_dir.join("Cargo.toml").is_file() {
            return Ok(0);
        }
        if !crate::util::which("cargo") {
            progress.line(&format!(
                "  WARNING: {} skipped (cargo not found)",
                self.label
            ));
            return Ok(0);
        }
        if !crate::builder::tools::musl_target_installed(self.triple) {
            progress.line(&format!(
                "  WARNING: {} skipped (rust target {} not installed; \
                 run `rustup target add {}` to enable)",
                self.label, self.triple, self.triple
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
        let out = cmd.output().context("failed to run cargo")?;
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

        let release_dir = self
            .rust_dir
            .join("target")
            .join(self.triple)
            .join("release");
        let mut installed = 0usize;
        for entry in std::fs::read_dir(&release_dir)?.flatten() {
            if let Some(bin) = crate::builder::testcase::rust_test_binary(&entry.path()) {
                let name = bin
                    .file_name()
                    .context("release-dir entry has no file name")?
                    .to_string_lossy()
                    .to_string();
                let group = self.groups.and_then(|g| g.get(&name));
                let dest_dir = match group {
                    Some(g) => self.dest.join(g),
                    None => self.dest.to_path_buf(),
                };
                std::fs::create_dir_all(&dest_dir)?;
                std::fs::copy(&bin, dest_dir.join(&name))
                    .with_context(|| format!("copy {} failed", bin.display()))?;
                crate::util::set_executable(&dest_dir.join(&name))?;
                match group {
                    Some(g) => progress.line(&format!("{} {g}/{name}", self.item_prefix)),
                    None => progress.line(&format!("{} {name}", self.item_prefix)),
                }
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
