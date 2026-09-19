//! 用例编译与安装。C 路径（build-initrd.sh install_testcases + testcases/Makefile
//! 的 Rust 接管）与 Rust 路径（Phase 3 的 no_std testfw 框架）并存：
//! `-static` 约束在 CMakeLists 与 rust/.cargo/config.toml 中各自冻结 —— VM 内
//! 无动态加载器，**不可放松**；交叉编译由 CMake 工具链 / cargo 负责。

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Progress;

/// 编译 testcases 并把可执行文件装入 `<rootfs>/tests/`。
/// 返回装入的二进制名列表。构建日志语义与脚本一致：
/// 失败时提取 `error:` 行（否则全文）作为错误信息；成功时打印 `warning:` 行。
pub fn install(testcases_dir: &Path, dest_tests: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    progress.line("Building testcases...");
    std::fs::create_dir_all(dest_tests)?;
    if !testcases_dir.is_dir() {
        progress.line("  WARNING: no testcases dir");
        return Ok(());
    }

    let build_dir = testcases_dir.join("build");
    std::fs::create_dir_all(&build_dir)?;
    let log_path = testcases_dir.join(".build.log");

    let cmake = std::process::Command::new("cmake")
        .arg("..")
        .current_dir(&build_dir)
        .output()
        .context("cmake 启动失败（安装 cmake）")?;
    let make = std::process::Command::new("make")
        .current_dir(&build_dir)
        .output()
        .context("make 启动失败")?;
    std::fs::write(&log_path, format!("{}{}", String::from_utf8_lossy(&cmake.stdout), String::from_utf8_lossy(&cmake.stderr)))?;
    let make_log = format!("{}{}", String::from_utf8_lossy(&make.stdout), String::from_utf8_lossy(&make.stderr));
    std::fs::write(&log_path, &make_log)?;

    if !cmake.status.success() || !make.status.success() {
        let shown: Vec<&str> = make_log
            .lines()
            .filter(|l| l.contains("error:"))
            .collect();
        let body = if shown.is_empty() { make_log.clone() } else { shown.join("\n") };
        anyhow::bail!("Testcases build failed\n{body}");
    }
    for line in make_log.lines().filter(|l| l.contains("warning:")) {
        progress.line(line);
    }
    let _ = std::fs::remove_file(&log_path);

    let bin_dir = build_dir.join("bin");
    if bin_dir.is_dir() {
        for entry in std::fs::read_dir(&bin_dir)?.flatten() {
            let p = entry.path();
            let is_exec = p.is_file() && is_executable(&p);
            if is_exec {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                std::fs::copy(&p, dest_tests.join(&name))?;
                common::fsutil::set_executable(&dest_tests.join(&name))?;
                progress.line(&format!("  Test: {name}"));
            }
        }
    }
    Ok(())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// 编译 no_std Rust 用例（`testcases/rust/` 独立 workspace）并装入
/// `<rootfs>/tests/`。产物走裸 syscall 静态链接（无 libc 依赖），标记协议
/// 与 C 用例完全一致，init 自动发现。
///
/// 降级规则（与禁止掩盖失败的冻结约束不冲突——降级是显式 WARN，不是吞错）：
/// - 目录缺失 → 静默跳过（未采用 Rust 用例的项目不受影响）；
/// - 交叉架构（宿主无对应 rust-std）或 cargo 缺失 → WARN 跳过；
/// - 构建失败 → **bail**（与 C 路径同语义，不静默）。
pub fn install_rust(
    rust_dir: &Path,
    dest_tests: &Path,
    arch: common::Arch,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    if !rust_dir.join("Cargo.toml").is_file() {
        return Ok(());
    }
    let host = common::Arch::parse(std::env::consts::ARCH);
    if host != Some(arch) {
        progress.line(&format!(
            "  WARNING: rust testcases skipped (host rust-std covers {}, target {} needs cross rust-std; \
             install via `rustup target add <triple>` to enable)",
            host.map(|a| a.name()).unwrap_or(std::env::consts::ARCH),
            arch.name()
        ));
        return Ok(());
    }
    if !common::fsutil::which("cargo") {
        progress.line("  WARNING: rust testcases skipped (cargo not found)");
        return Ok(());
    }

    progress.line("Building rust testcases...");
    let target_dir = rust_dir.join("target");
    let out = std::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(rust_dir)
        .output()
        .context("cargo 启动失败")?;
    if !out.status.success() {
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        anyhow::bail!("Rust testcases build failed\n{log}");
    }

    let mut installed = 0usize;
    for entry in std::fs::read_dir(target_dir.join("release"))?.flatten() {
        if let Some(bin) = rust_test_binary(&entry.path()) {
            let name = bin.file_name().unwrap().to_string_lossy().to_string();
            std::fs::copy(&bin, dest_tests.join(&name))
                .with_context(|| format!("拷贝 {} 失败", bin.display()))?;
            common::fsutil::set_executable(&dest_tests.join(&name))?;
            progress.line(&format!("  Test (rust): {name}"));
            installed += 1;
        }
    }
    if installed == 0 {
        anyhow::bail!("Rust testcases build succeeded but no binaries found under {}/release", target_dir.display());
    }
    Ok(())
}

/// `target/release/` 下的测试二进制判定：可执行文件、排除 `.d`/`.rlib`/`.so` 等副产物。
fn rust_test_binary(p: &Path) -> Option<PathBuf> {
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
        let bin = mk("test-rs-example", 0o755);
        let dep = mk("test-rs-example.d", 0o644);
        let rlib = mk("libtestfw.rlib", 0o644);
        let script = mk("build_script_build-xxxx", 0o755);

        assert!(rust_test_binary(&bin).is_some());
        assert!(rust_test_binary(&dep).is_none(), ".d 副产物必须被过滤");
        assert!(rust_test_binary(&rlib).is_none(), "rlib 必须被过滤");
        assert!(rust_test_binary(&script).is_none(), "build script 必须被过滤");
        std::fs::remove_dir_all(&dir).ok();
    }
}
