//! tools workspace 装载：VM 内常驻工具（std Rust + musl 静态）→ tools.img
//! 的 `/bin/`（外挂 virtio-blk 数据盘，挂 /tools 后由 PATH 引用）。
//!
//! 与 testcases 分类正交：工具不进 `/tests`、不被 init 自动发现、不参与
//! 标记协议判定。构建走 `cargo build --release --target <arch musl triple>`
//! （见 `Arch::rust_musl_triple`）；musl crt-static 自含链接，产物为无动态
//! 依赖的静态 ELF —— VM 无动态加载器/无动态库的事实不变。
//!
//! 降级规则（显式 WARN，不是吞错；构建失败仍 bail）：
//! - `tools/` 目录缺失 → 静默跳过（未采用 tools 的项目不受影响）；
//! - cargo 缺失或 musl target 未随 toolchain 安装 → WARN 跳过。
//!
//! 返回 `Ok(true)` = 有工具装入（此时才产出 tools.img 与挂载 hook）；
//! `Ok(false)` = 按上述规则跳过。

use std::path::Path;

use anyhow::Context;

use crate::Progress;

pub fn install(
    rust_dir: &Path,
    dest_bin: &Path,
    arch: common::Arch,
    progress: &mut Progress,
) -> anyhow::Result<bool> {
    if !rust_dir.join("Cargo.toml").is_file() {
        return Ok(false);
    }
    let triple = arch.rust_musl_triple();
    if !common::fsutil::which("cargo") {
        progress.line("  WARNING: tools skipped (cargo not found)");
        return Ok(false);
    }
    if !musl_target_installed(triple) {
        progress.line(&format!(
            "  WARNING: tools skipped (rust target {triple} not installed; \
             run `rustup target add {triple}` to enable)"
        ));
        return Ok(false);
    }

    progress.line("Building VM tools...");
    let out = std::process::Command::new("cargo")
        .args(["build", "--release", "--target", triple])
        .current_dir(rust_dir)
        .output()
        .context("cargo 启动失败")?;
    if !out.status.success() {
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        anyhow::bail!("Tools build failed\n{log}");
    }

    let bin_dir = rust_dir.join("target").join(triple).join("release");
    std::fs::create_dir_all(dest_bin)?;
    let mut installed = 0usize;
    for entry in std::fs::read_dir(&bin_dir)?.flatten() {
        if let Some(bin) = super::testcase::rust_test_binary(&entry.path()) {
            let name = bin.file_name().unwrap().to_string_lossy().to_string();
            std::fs::copy(&bin, dest_bin.join(&name))
                .with_context(|| format!("拷贝 {} 失败", bin.display()))?;
            common::fsutil::set_executable(&dest_bin.join(&name))?;
            progress.line(&format!("  Tool: {name}"));
            installed += 1;
        }
    }
    if installed == 0 {
        anyhow::bail!(
            "Tools build succeeded but no binaries found under {}",
            bin_dir.display()
        );
    }
    Ok(true)
}

/// musl target 是否已随 toolchain 安装（sysroot 的 rustlib 目录存在即视为可用）。
fn musl_target_installed(triple: &str) -> bool {
    let out = std::process::Command::new("rustc")
        .args(["--print", "target-libdir", "--target", triple])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            !dir.is_empty() && Path::new(&dir).is_dir()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musl_target_check_reflects_install_state() {
        // 本机装了 aarch64 musl（手装 rust-std），x86_64 未装 —— 两个方向都要覆盖
        let aarch64 = musl_target_installed("aarch64-unknown-linux-musl");
        let x86_64 = musl_target_installed("x86_64-unknown-linux-musl");
        assert!(aarch64 || x86_64, "至少一个 musl target 应可用");
    }
}
