//! builder — 构建器。Phase 2 从 `build-initrd.sh` / `fetch-busybox.sh`
//! 接管：BusyBox 供给、C 用例编译、initramfs/rootfs 组装（行为等价移植，
//! 消息文本与退出码语义对齐 shell 基线）。

pub mod busybox;
pub mod image;
pub mod modconf;
pub mod testcase;
pub mod tools;
pub mod verify;

use std::path::Path;

use anyhow::Context;

/// VM 内 init 的声明式注入钩子：片段插入 mount 之后、insmod 之前。
/// 对应 rootfs 内的 `/init-hooks.sh`（缺省不存在，init 侧有守卫 source）。
#[derive(Debug, Clone)]
pub struct InitHook {
    pub name: String,
    pub script: String,
}

impl InitHook {
    pub fn shell(name: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            script: script.into(),
        }
    }
}

/// 进度输出：同时打印到终端，可选追加到运行工件 build.log。
pub struct Progress {
    log: Option<std::fs::File>,
}

impl Progress {
    pub fn stdout() -> Self {
        Self { log: None }
    }

    pub fn with_log(path: &Path) -> anyhow::Result<Self> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { log: Some(log) })
    }

    pub fn line(&mut self, msg: &str) {
        println!("{msg}");
        if let Some(f) = self.log.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// 构建两段式引导对（build-initrd.sh 的 Rust 接管）：
/// `infra/initrd.img`（initramfs：busybox + modules-boot.conf + init-initramfs）
/// 与 `infra/rootfs.img`（ext4：busybox + modules.conf + init + /tests）。
pub fn build_boot_pair(
    infra_dir: &Path,
    kernel_path: &Path,
    arch: common::Arch,
    supply: &busybox::Supply,
    hooks: &[InitHook],
    progress: &mut Progress,
) -> anyhow::Result<()> {
    // 与脚本一致的先决检查
    if !kernel_path.join("arch").is_dir() {
        anyhow::bail!(
            "Cannot find kernel source directory at {}\nSet KERNEL_PATH environment variable to specify the location:\n  KERNEL_PATH=/path/to/kernel cargo xtask build",
            kernel_path.display()
        );
    }
    if !common::fsutil::which("mke2fs") {
        anyhow::bail!("mke2fs not found (install e2fsprogs)");
    }

    let busybox_bin = busybox::ensure(infra_dir, arch, supply, progress)?;

    // ---------- initrd.img: minimal initramfs ----------
    progress.line("Building initrd.img (minimal initramfs)...");
    let initramfs_dir = infra_dir.join("initramfs");
    assemble_busybox_tree(&initramfs_dir, &busybox_bin)?;
    std::fs::copy(infra_dir.join("init-initramfs"), initramfs_dir.join("init"))
        .context("复制 init-initramfs 失败")?;
    common::fsutil::set_executable(&initramfs_dir.join("init"))?;
    std::fs::create_dir_all(initramfs_dir.join("mnt"))?;
    std::fs::create_dir_all(initramfs_dir.join("lib/modules"))?;
    modconf::copy_modules(
        &initramfs_dir.join("lib/modules"),
        &infra_dir.join("modules-boot.conf"),
        kernel_path,
        infra_dir,
        progress,
    )?;
    image::pack_initramfs(&initramfs_dir, &infra_dir.join("initrd.img"))?;

    // ---------- rootfs.img: ext4 rootfs with tests ----------
    progress.line("Building rootfs.img (ext4 rootfs)...");
    let rootfs_dir = infra_dir.join("rootfs");
    assemble_busybox_tree(&rootfs_dir, &busybox_bin)?;
    std::fs::copy(infra_dir.join("init"), rootfs_dir.join("init")).context("复制 init 失败")?;
    common::fsutil::set_executable(&rootfs_dir.join("init"))?;
    std::fs::create_dir_all(rootfs_dir.join("lib/modules"))?;
    modconf::copy_modules(
        &rootfs_dir.join("lib/modules"),
        &infra_dir.join("modules.conf"),
        kernel_path,
        infra_dir,
        progress,
    )?;
    write_hooks(&rootfs_dir, hooks)?;
    testcase::install(
        &infra_dir.join("testcases"),
        &rootfs_dir.join("tests"),
        progress,
    )?;
    testcase::install_rust(
        &infra_dir.join("testcases/rust"),
        &rootfs_dir.join("tests"),
        arch,
        progress,
    )?;
    // tools workspace（常驻工具）→ /bin：与用例分类正交，见 tools::install
    tools::install(
        &infra_dir.join("tools"),
        &rootfs_dir.join("bin"),
        arch,
        progress,
    )?;
    image::make_ext4(&rootfs_dir, &infra_dir.join("rootfs.img"))?;

    progress.line("");
    progress.line("Done:");
    for f in ["initrd.img", "rootfs.img"] {
        let p = infra_dir.join(f);
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        progress.line(&format!("  {} {}", f, common::fmt::human_size_ls(size)));
    }
    Ok(())
}

/// busybox 用户land 组装：静态二进制 + applet 符号链接 + 骨架目录 + root 账户。
/// 注意：applet 安装会执行 busybox 二进制，交叉组装需宿主同构（与脚本一致）。
pub fn assemble_busybox_tree(dest: &Path, busybox_bin: &Path) -> anyhow::Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    for d in [
        "bin",
        "sbin",
        "usr/bin",
        "usr/sbin",
        "proc",
        "sys",
        "dev",
        "tmp",
        "mnt",
        "etc/init.d",
        "var/run",
        "root",
    ] {
        std::fs::create_dir_all(dest.join(d))?;
    }
    std::fs::copy(busybox_bin, dest.join("bin/busybox"))?;
    common::fsutil::set_executable(&dest.join("bin/busybox"))?;

    let status = std::process::Command::new("./busybox")
        .args(["--install", "-s", "."])
        .current_dir(dest.join("bin"))
        .status()
        .context("执行 busybox --install 失败（交叉组装需宿主同构）")?;
    if !status.success() {
        anyhow::bail!("busybox --install failed");
    }

    // 绝对符号链接改写（busybox 可能指向 /usr/bin/...）
    for sub in ["bin", "sbin", "usr/bin", "usr/sbin"] {
        let dir = dest.join(sub);
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let p = entry.path();
            if p.is_symlink() {
                if let Ok(target) = std::fs::read_link(&p) {
                    if target.is_absolute() {
                        let name = target.file_name().unwrap_or_default();
                        let _ = std::fs::remove_file(&p);
                        std::os::unix::fs::symlink(name, &p)?;
                    }
                }
            }
        }
    }

    std::fs::write(dest.join("etc/passwd"), "root:x:0:0:root:/root:/bin/sh\n")?;
    std::fs::write(dest.join("etc/group"), "root:x:0:\n")?;
    Ok(())
}

fn write_hooks(rootfs_dir: &Path, hooks: &[InitHook]) -> anyhow::Result<()> {
    if hooks.is_empty() {
        return Ok(());
    }
    let mut body = String::from("# generated by builder (InitHook)\n");
    for h in hooks {
        body.push_str(&format!("# --- hook: {} ---\n{}\n", h.name, h.script));
    }
    std::fs::write(rootfs_dir.join("init-hooks.sh"), body)?;
    Ok(())
}
