//! 用例编译与安装。C/Rust 用例同走一个 cargo workspace
//! （`infra/testcases/`）：Rust 框架为入口，C 测试体由用例 crate 的
//! build.rs（cc crate）编入同一二进制（C over Rust）。产物是 musl 静态
//! ELF——VM 内无动态加载器，**-static 约束不可放松**；交叉编译：Rust 恒
//! `--target <arch musl triple>`（rustflags 不挂文件，宿主 OS 无关），
//! C 的编译器由 `cross::CrossSetup` 以 `CC_<TRIPLE>` 注入（zig cc 包装）。
//!
//! 降级规则（与禁止掩盖失败的冻结约束不冲突——降级是显式 WARN，不是吞错）：
//! - 目录缺失 → 静默跳过（未采用 Rust 用例的项目不受影响）；
//! - cargo 缺失或 musl target 未随 toolchain 安装 → WARN 跳过；
//! - 构建失败 → **bail**（不静默）。

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::cross::CrossSetup;
use crate::Progress;

/// 编译用例 workspace（`infra/testcases/`）并装入 `<rootfs>/tests/`。
/// 返回装入的二进制名列表无必要——init 自动发现，这里只保证产物齐全。
/// 构建日志语义：失败时全文 bail（cargo 已含 `error:` 行），成功时打印
/// `warning:` 行。
pub fn install_rust(
    rust_dir: &Path,
    dest_tests: &Path,
    arch: common::Arch,
    cross: &CrossSetup,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    if !rust_dir.join("Cargo.toml").is_file() {
        return Ok(());
    }
    if !common::fsutil::which("cargo") {
        progress.line("  WARNING: rust testcases skipped (cargo not found)");
        return Ok(());
    }
    let triple = arch.rust_musl_triple();
    if !crate::tools::musl_target_installed(triple) {
        progress.line(&format!(
            "  WARNING: rust testcases skipped (rust target {triple} not installed; \
             run `rustup target add {triple}` to enable)"
        ));
        return Ok(());
    }

    progress.line("Building testcases (cargo: rust entry + C bodies via cc)...");
    std::fs::create_dir_all(dest_tests)?;
    let target_dir = rust_dir.join("target");
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["build", "--release", "--target", triple])
        .current_dir(rust_dir);
    cross.apply_to_cargo(&mut cmd);
    let out = cmd.output().context("cargo 启动失败")?;
    if !out.status.success() {
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        anyhow::bail!("Rust testcases build failed\n{log}");
    }
    for line in String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("warning:"))
    {
        progress.line(line);
    }

    let mut installed = 0usize;
    let release_dir = target_dir.join(triple).join("release");
    for entry in std::fs::read_dir(&release_dir)?.flatten() {
        if let Some(bin) = rust_test_binary(&entry.path()) {
            let name = bin.file_name().unwrap().to_string_lossy().to_string();
            std::fs::copy(&bin, dest_tests.join(&name))
                .with_context(|| format!("拷贝 {} 失败", bin.display()))?;
            common::fsutil::set_executable(&dest_tests.join(&name))?;
            progress.line(&format!("  Test: {name}"));
            installed += 1;
        }
    }
    if installed == 0 {
        anyhow::bail!(
            "Rust testcases build succeeded but no binaries found under {}",
            release_dir.display()
        );
    }
    Ok(())
}

/// `target/release/` 下的测试二进制判定：可执行文件、排除 `.d`/`.rlib`/`.so` 等副产物。
pub(crate) fn rust_test_binary(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_string_lossy();
    if !p.is_file() || !is_executable(p) {
        return None;
    }
    let skip = name.ends_with(".d")
        || name.ends_with(".rlib")
        || name.ends_with(".so")
        || name.ends_with(".a")
        || name.ends_with(".rmeta")
        || name.contains("build_script");
    (!skip).then(|| p.to_path_buf())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_binary_filter_rejects_build_artifacts() {
        let dir = std::env::temp_dir().join(format!("builder-tc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, mode: u32| {
            let p = dir.join(name);
            std::fs::write(&p, b"elf").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        let bin = mk("test-example", 0o755);
        let dep = mk("test-example.d", 0o644);
        let rlib = mk("libtestfw.rlib", 0o644);
        let script = mk("build_script_build-xxxx", 0o755);

        assert!(rust_test_binary(&bin).is_some());
        assert!(rust_test_binary(&dep).is_none(), ".d 副产物必须被过滤");
        assert!(rust_test_binary(&rlib).is_none(), "rlib 必须被过滤");
        assert!(
            rust_test_binary(&script).is_none(),
            "build script 必须被过滤"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
