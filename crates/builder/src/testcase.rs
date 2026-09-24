//! 用例编译与安装。C 路径（build-initrd.sh install_testcases + testcases/Makefile
//! 的 Rust 接管）与 Rust 路径（Phase 3 的 no_std testfw 框架）并存：
//! `-static` 约束在 CMakeLists 与 rust/.cargo/config.toml 中各自冻结 —— VM 内
//! 无动态加载器，**不可放松**。交叉编译：C 走 `cross::CrossSetup` 的
//! CMAKE_C_COMPILER（非 Linux 宿主 = zig cc 包装）；Rust 恒以
//! `--target <arch musl triple>` 构建（rustflags 挂在 triple 上，宿主 OS 无关）。

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::cross::CrossSetup;
use crate::Progress;

/// 编译 testcases 并把可执行文件装入 `<rootfs>/tests/`。
/// 返回装入的二进制名列表。构建日志语义与脚本一致：
/// 失败时提取 `error:` 行（否则全文）作为错误信息；成功时打印 `warning:` 行。
pub fn install(
    testcases_dir: &Path,
    dest_tests: &Path,
    cross: &CrossSetup,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    progress.line("Building testcases...");
    std::fs::create_dir_all(dest_tests)?;
    if !testcases_dir.is_dir() {
        progress.line("  WARNING: no testcases dir");
        return Ok(());
    }

    let build_dir = testcases_dir.join("build");
    std::fs::create_dir_all(&build_dir)?;
    let log_path = testcases_dir.join(".build.log");

    let mut cmake = std::process::Command::new("cmake");
    cmake.arg("..").current_dir(&build_dir);
    if let Some(cc) = &cross.cc_wrapper {
        // 交叉：包装脚本即 C 编译器（zig cc -target <triple>）。必须声明目标
        // 系统，否则 Darwin 宿主的 CMake 按本机编译器探测注入 `-arch` 等
        // Apple 旗标，zig cc 以 linux 目标拒绝。
        cmake.arg("-DCMAKE_SYSTEM_NAME=Linux");
        let triple = cross.target_triple();
        cmake.arg(format!("-DCMAKE_SYSTEM_PROCESSOR={}", triple.split('-').next().unwrap_or("")));
        cmake.arg(format!("-DCMAKE_C_COMPILER={}", cc.display()));
    }
    let cmake = cmake.output().context("cmake 启动失败（安装 cmake）")?;
    let make = std::process::Command::new("make")
        .current_dir(&build_dir)
        .output()
        .context("make 启动失败")?;
    let make_log = format!(
        "{}{}",
        String::from_utf8_lossy(&make.stdout),
        String::from_utf8_lossy(&make.stderr)
    );
    std::fs::write(
        &log_path,
        format!(
            "-- cmake --\n{}{}\n-- make --\n{}",
            String::from_utf8_lossy(&cmake.stdout),
            String::from_utf8_lossy(&cmake.stderr),
            make_log
        ),
    )?;

    if !cmake.status.success() || !make.status.success() {
        let shown: Vec<&str> = make_log.lines().filter(|l| l.contains("error:")).collect();
        let body = if shown.is_empty() {
            make_log.clone()
        } else {
            shown.join("\n")
        };
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
    std::fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// 编译 no_std Rust 用例（`testcases/rust/` 独立 workspace）并装入
/// `<rootfs>/tests/`。产物走裸 syscall 静态链接（无 libc 依赖），标记协议
/// 与 C 用例完全一致，init 自动发现。
///
/// 恒以 `--target <arch musl triple>` 构建（宿主 OS 无关；rustflags 见
/// rust/.cargo/config.toml 的显式 triple 段）。降级规则（与禁止掩盖失败的
/// 冻结约束不冲突——降级是显式 WARN，不是吞错）：
/// - 目录缺失 → 静默跳过（未采用 Rust 用例的项目不受影响）；
/// - cargo 缺失或 musl target 未随 toolchain 安装 → WARN 跳过；
/// - 构建失败 → **bail**（与 C 路径同语义，不静默）。
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

    progress.line("Building rust testcases...");
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

    let mut installed = 0usize;
    let release_dir = target_dir.join(triple).join("release");
    for entry in std::fs::read_dir(&release_dir)?.flatten() {
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
        assert!(
            rust_test_binary(&script).is_none(),
            "build script 必须被过滤"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
