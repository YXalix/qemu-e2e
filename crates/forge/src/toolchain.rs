//! 钉死工具链镜像的容器调用：镜像供给（pull 回落本地构建 Dockerfile.kernel）、
//! 通用 docker run 封装、架构 → make 环境 / clangd triple / make 镜像目标
//! 映射（照 kernel.sh 原值）。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;
use common::Arch;

use crate::{volume, Progress};

/// 工具链镜像覆盖 env。注意不能叫 `KERNEL_IMAGE`——那个名字已被 CLI 配置面
/// 占用（启动内核镜像覆盖，config.rs）。
pub const IMAGE_ENV: &str = "KERNEL_TOOLCHAIN_IMAGE";

/// 工具链镜像解析：env `KERNEL_TOOLCHAIN_IMAGE` > ghcr 发布镜像。devcontainer
/// 渲染与此同源（保证编辑容器与构建容器同镜像）。
pub fn resolve_image() -> String {
    std::env::var(IMAGE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::DEFAULT_IMAGE.to_string())
}

/// 工具链镜像是否在位（引擎权威）。
pub fn image_present(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 镜像供给：在位即短路；pull 失败回落本地构建 devkit/docker/Dockerfile.kernel
/// （与 kernel-builder.yml 发布 ghcr 的钉死工具链同源）。
pub fn ensure_image(
    image: &str,
    dockerfile_dir: &Path,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    volume::engine_guard()?;
    if image_present(image) {
        return Ok(());
    }
    progress.line(&format!("Kernel toolchain: pulling {image} ..."));
    let pulled = Command::new("docker")
        .arg("pull")
        .arg(image)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if pulled {
        return Ok(());
    }
    let dockerfile = dockerfile_dir.join("Dockerfile.kernel");
    progress.line(&format!(
        "Kernel toolchain: pull failed, building locally from {} ...",
        dockerfile.display()
    ));
    let st = Command::new("docker")
        .arg("build")
        .arg("-t")
        .arg(image)
        .arg("-f")
        .arg(&dockerfile)
        .arg(dockerfile_dir)
        .status()
        .context("failed to run docker build")?;
    anyhow::ensure!(
        st.success(),
        "local toolchain-image build failed ({dockerfile:?})"
    );
    Ok(())
}

/// 通用容器调用基座：volume 挂 /ksrc（workspace），工作目录 /ksrc。
fn base_cmd(volume: &str, image: &str, tty: bool) -> Command {
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm", "-i"]);
    if tty {
        cmd.arg("-t");
    }
    cmd.arg("--entrypoint=")
        .arg("-v")
        .arg(format!("{volume}:/ksrc"));
    cmd.arg("-w").arg("/ksrc").arg(image);
    cmd
}

/// 流式执行容器内脚本（stdio 继承，内核构建的长输出实时可见）。
pub fn run_streaming(
    volume: &str,
    image: &str,
    envs: &[(&str, &str)],
    script: &str,
) -> anyhow::Result<()> {
    let mut cmd = base_cmd(volume, image, false);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.arg("sh").arg("-c").arg(script);
    let st = cmd
        .status()
        .with_context(|| format!("failed to run docker (is the engine up? volume {volume})"))?;
    anyhow::ensure!(
        st.success(),
        "容器内命令失败（exit {}）：{script}",
        st.code().unwrap_or(-1)
    );
    Ok(())
}

/// 交互式容器命令（shell：TTY + stdio 继承），返回退出码。
pub fn run_tty(
    volume: &str,
    image: &str,
    envs: &[(&str, &str)],
    argv: &[&str],
) -> anyhow::Result<i32> {
    let mut cmd = base_cmd(volume, image, true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.args(argv);
    let st = cmd
        .status()
        .with_context(|| format!("failed to run docker (is the engine up? volume {volume})"))?;
    Ok(st.code().unwrap_or(130))
}

/// 经 stdin 向容器内写文件（平台无关，不依赖宿主视图在位）。
pub fn write_file(volume: &str, image: &str, dest: &str, content: &str) -> anyhow::Result<()> {
    let mut child = base_cmd(volume, image, false)
        .arg("sh")
        .arg("-c")
        .arg(format!("cat > {dest}"))
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start docker")?;
    child
        .stdin
        .as_mut()
        .expect("stdin 已声明 piped")
        .write_all(content.as_bytes())
        .with_context(|| format!("write inside container to {dest} failed"))?;
    let st = child.wait()?;
    anyhow::ensure!(
        st.success(),
        "in-container write to {dest} failed (exit {})",
        st.code().unwrap_or(-1)
    );
    Ok(())
}

/// arch → 容器内 make 环境（arm64 在 arm64 容器 = 原生前端，其余交叉；
/// 照 kernel.sh 原值——容器基座是 arm64，x86_64/riscv64 无条件交叉）。
pub fn make_env(arch: Arch) -> Vec<(&'static str, &'static str)> {
    match arch {
        Arch::Arm64 => vec![("ARCH", "arm64")],
        Arch::X86_64 => vec![("ARCH", "x86_64"), ("CROSS_COMPILE", "x86_64-linux-gnu-")],
        Arch::Riscv64 => vec![("ARCH", "riscv64"), ("CROSS_COMPILE", "riscv64-linux-gnu-")],
    }
}

/// arch → clangd --target triple（.clangd 的 --target 行替换值）。
pub fn clangd_target(arch: Arch) -> &'static str {
    match arch {
        Arch::Arm64 => "aarch64-linux-gnu",
        Arch::X86_64 => "x86_64-linux-gnu",
        Arch::Riscv64 => "riscv64-linux-gnu",
    }
}

/// arch → make 的内核镜像目标（x86_64 无 Image 目标，只有 bzImage）。
pub fn make_image_target(arch: Arch) -> &'static str {
    match arch {
        Arch::Arm64 | Arch::Riscv64 => "Image",
        Arch::X86_64 => "bzImage",
    }
}

/// POSIX 单引号转义（宿主视图路径 / URL / ref 进容器 `sh -c` 用）。
pub use common::shell::quote as shell_quote;

/// compile_commands.json 生成（/ksrc 原始形态，容器内 clangd/devcontainer
/// 消费）。脚本由内核树自带（从 .cmd 文件聚合，无需 bear）：mainline/
/// openEuler 均为 `scripts/clang-tools/gen_compile_commands.py`；个别树可能
/// 放在 `scripts/compile_commands.py`，作回落。
pub fn cdb_generate(volume: &str, image: &str) -> anyhow::Result<()> {
    let script = "if [ -f scripts/clang-tools/gen_compile_commands.py ]; then \
                      python3 scripts/clang-tools/gen_compile_commands.py; \
                  else \
                      python3 scripts/compile_commands.py; \
                  fi";
    run_streaming(volume, image, &[], script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_env_table_matches_kernel_sh() {
        assert_eq!(make_env(Arch::Arm64), vec![("ARCH", "arm64")]);
        assert_eq!(
            make_env(Arch::X86_64),
            vec![("ARCH", "x86_64"), ("CROSS_COMPILE", "x86_64-linux-gnu-")]
        );
        assert_eq!(
            make_env(Arch::Riscv64),
            vec![("ARCH", "riscv64"), ("CROSS_COMPILE", "riscv64-linux-gnu-")]
        );
    }

    #[test]
    fn clangd_target_table() {
        assert_eq!(clangd_target(Arch::Arm64), "aarch64-linux-gnu");
        assert_eq!(clangd_target(Arch::X86_64), "x86_64-linux-gnu");
        assert_eq!(clangd_target(Arch::Riscv64), "riscv64-linux-gnu");
    }

    #[test]
    fn make_image_target_table() {
        assert_eq!(make_image_target(Arch::Arm64), "Image");
        assert_eq!(make_image_target(Arch::X86_64), "bzImage");
        assert_eq!(make_image_target(Arch::Riscv64), "Image");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        // 语义钉在 common::shell（shlex.quote：安全字符原样，其余整体包裹）
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("/home/u/ksrc"), "/home/u/ksrc");
    }
}
