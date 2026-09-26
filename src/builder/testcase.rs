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

use crate::builder::cross::CrossSetup;
use crate::util::Progress;

/// 编译用例 workspace（`infra/testcases/`）并装入 `<rootfs>/tests/`。
/// 返回装入的二进制名列表无必要——init 自动发现，这里只保证产物齐全。
/// 构建日志语义：失败时全文 bail（cargo 已含 `error:` 行），成功时打印
/// `warning:` 行。
pub(crate) fn install_rust(
    rust_dir: &Path,
    dest_tests: &Path,
    arch: crate::Arch,
    cross: &CrossSetup,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let groups = group_map(rust_dir);
    let job = crate::builder::cargo_install::CargoInstall {
        rust_dir,
        dest: dest_tests,
        triple: arch.rust_musl_triple(),
        cross,
        label: "rust testcases",
        item_prefix: "  Test:",
        groups: Some(&groups),
    };
    // 返回 0 = workspace 缺失或显式 WARN 降级（cargo/musl target 缺失），不算错；
    // 构建成功但零产物由 CargoInstall::run 内部 bail。
    job.run(progress)?;
    Ok(())
}

/// 测试组映射：扫描 workspace 成员 crate 的 manifest，读
/// `package.metadata.virtuoso.group`（包名 + 自定义 `[[bin]]` 名 → 组）。
/// 无声明的 crate 平铺进 `/tests/`；非目录条目与坏 manifest 忽略。
pub(crate) fn group_map(rust_dir: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir(rust_dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(v) = text.parse::<toml::Table>() else {
            continue;
        };
        let Some(group) = v
            .get("package")
            .and_then(|p| p.get("metadata"))
            .and_then(|m| m.get("virtuoso"))
            .and_then(|x| x.get("group"))
            .and_then(|g| g.as_str())
        else {
            continue;
        };
        let mut names: Vec<String> = v
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default();
        if let Some(bins) = v.get("bin").and_then(|b| b.as_array()) {
            for bin in bins {
                if let Some(n) = bin.get("name").and_then(|x| x.as_str()) {
                    names.push(n.to_string());
                }
            }
        }
        for name in names {
            map.insert(name, group.to_string());
        }
    }
    map
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
        let rlib = mk("libcoda.rlib", 0o644);
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

    #[test]
    fn group_map_reads_metadata_and_bin_names() {
        let dir = std::env::temp_dir().join(format!("builder-gmap-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("test-net")).unwrap();
        std::fs::create_dir_all(dir.join("test-plain")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(
            dir.join("test-net/Cargo.toml"),
            "[package]\nname = \"test-net\"\n\n[[bin]]\nname = \"net-bin\"\n\n[package.metadata.virtuoso]\ngroup = \"smoke\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("test-plain/Cargo.toml"),
            "[package]\nname = \"test-plain\"\n",
        )
        .unwrap();
        let map = group_map(&dir);
        assert_eq!(map.get("test-net").map(String::as_str), Some("smoke"));
        assert_eq!(map.get("net-bin").map(String::as_str), Some("smoke"));
        assert!(!map.contains_key("test-plain"), "无声明 → 平铺，不进映射");
        std::fs::remove_dir_all(&dir).ok();
    }
}
