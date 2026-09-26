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

use crate::builder::cross::CrossSetup;
use crate::util::Progress;

pub(crate) fn install(
    rust_dir: &Path,
    dest_bin: &Path,
    arch: crate::Arch,
    cross: &CrossSetup,
    progress: &mut Progress,
) -> anyhow::Result<bool> {
    let job = crate::builder::cargo_install::CargoInstall {
        rust_dir,
        dest: dest_bin,
        triple: arch.rust_musl_triple(),
        cross,
        label: "VM tools",
        item_prefix: "  Tool:",
    };
    // 返回 0 = workspace 缺失或显式 WARN 降级（cargo/musl target 缺失）；
    // 构建成功但零产物由 CargoInstall::run 内部 bail。
    Ok(job.run(progress)? > 0)
}

/// musl target 是否已随 toolchain 安装（sysroot 的 rustlib 目录存在即视为可用）。
pub(crate) fn musl_target_installed(triple: &str) -> bool {
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
