//! volume 的宿主平台视图与引擎守卫。
//!
//! 源码权威存 named volume，宿主经平台视图直接读写：macOS 走 OrbStack 视图
//! （Docker Desktop 的 volume 在 VM 虚拟盘里，宿主不可见）；Linux 上 volume
//! 本体就在宿主文件系统，取引擎权威 Mountpoint（rootless 落 $HOME 下，免
//! root）。两个平台该路径都是纯宿主路径——build/cc 产出的 compile_commands.json
//! 据此改写，宿主 clangd 直接消费。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

/// OrbStack 对 named volume 的宿主视图（OrbStack 运行时存在）。
pub fn orbstack_view(home: &Path, volume: &str) -> PathBuf {
    home.join("OrbStack").join("docker").join("volumes").join(volume)
}

/// volume 的宿主可见路径。
pub fn host_view(volume: &str) -> anyhow::Result<PathBuf> {
    match std::env::consts::OS {
        "macos" => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME 未设置，无法定位 OrbStack volume 视图")?;
            Ok(orbstack_view(&home, volume))
        }
        "linux" => {
            let out = Command::new("docker")
                .args(["volume", "inspect", volume, "--format", "{{ .Mountpoint }}"])
                .output()
                .context("运行 docker volume inspect 失败（docker 在位？）")?;
            anyhow::ensure!(
                out.status.success(),
                "volume {volume} 不存在（先 `virtuoso kernel clone <git-url>`）：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            let mp = String::from_utf8_lossy(&out.stdout).trim().to_string();
            anyhow::ensure!(!mp.is_empty(), "volume {volume} 的 Mountpoint 为空");
            Ok(PathBuf::from(mp))
        }
        os => anyhow::bail!(
            "不支持的宿主平台 {os}（docker 内核供给支持 macOS/Linux；其他平台用 `virtuoso kernel export` 最小树回退）"
        ),
    }
}

/// macOS 引擎守卫：volume 的宿主视图由 OrbStack 提供，docker 端点必须指向
/// OrbStack 引擎，否则 clone/build 会把卷建进别的引擎 VM，视图永远看不到。
/// 非 macOS 恒通过（Linux 的 volume 本体即宿主目录，无视图引擎问题）。
pub fn engine_guard() -> anyhow::Result<()> {
    if std::env::consts::OS != "macos" {
        return Ok(());
    }
    let ep = match std::env::var("DOCKER_HOST") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => docker_context_endpoint()?,
    };
    if endpoint_is_orbstack(&ep) {
        return Ok(());
    }
    anyhow::bail!(
        "macOS 上当前 docker 引擎不是 OrbStack（endpoint: {ep}）。\n  执行 'docker context use orbstack'，或对单条命令 export DOCKER_HOST=unix://$HOME/.orbstack/run/docker.sock。"
    )
}

/// 引擎端点是否指向 OrbStack（unix socket 是 `$HOME/.orbstack/...`；远程
/// context 主机名通常也含 orbstack 字样，宽松匹配宁可放行——误拦的代价是
/// 用户被守卫挡住）。
fn endpoint_is_orbstack(endpoint: &str) -> bool {
    endpoint.contains("orbstack")
}

/// 当前 docker context 的引擎端点（DOCKER_HOST 未设置时的权威来源）。
fn docker_context_endpoint() -> anyhow::Result<String> {
    let out = Command::new("docker")
        .args(["context", "show"])
        .output()
        .map_err(|_| anyhow::anyhow!("docker 不可用（安装并启动 OrbStack 后重试）"))?;
    anyhow::ensure!(
        out.status.success(),
        "docker context show 失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    anyhow::ensure!(!name.is_empty(), "docker context 为空（安装并启动 OrbStack 后重试）");
    let out = Command::new("docker")
        .args(["context", "inspect", "--format", "{{ .Endpoints.docker.Host }}", &name])
        .output()
        .context("运行 docker context inspect 失败")?;
    anyhow::ensure!(
        out.status.success(),
        "docker context inspect {name} 失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 引擎里的全部卷名（`docker volume ls`，引擎即权威注册表，不另建清单）。
pub fn list() -> anyhow::Result<Vec<String>> {
    let out = Command::new("docker")
        .args(["volume", "ls", "--format", "{{ .Name }}"])
        .output()
        .context("运行 docker volume ls 失败（docker 在位？）")?;
    anyhow::ensure!(
        out.status.success(),
        "docker volume ls 失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// volume 是否存在（引擎权威）。
pub fn exists(volume: &str) -> bool {
    Command::new("docker")
        .args(["volume", "inspect", volume])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 卷内容状态（fs-only 探测宿主视图，不起容器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 空 / 不可达
    Empty,
    /// 有源码树（Makefile），未配置（无 .config）
    Cloned,
    /// Makefile + .config 齐备，可构建
    Configured,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Empty => "empty",
            Status::Cloned => "cloned",
            Status::Configured => "configured",
        }
    }
}

/// 按宿主视图探测卷内容：Makefile+.config = configured，仅 Makefile = cloned。
pub fn status(view: &Path) -> Status {
    match (view.join("Makefile").is_file(), view.join(".config").is_file()) {
        (true, true) => Status::Configured,
        (true, false) => Status::Cloned,
        _ => Status::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbstack_view_layout() {
        let p = orbstack_view(Path::new("/Users/u"), "ksrc-oe66");
        assert_eq!(p, PathBuf::from("/Users/u/OrbStack/docker/volumes/ksrc-oe66"));
    }

    #[test]
    fn engine_endpoint_matching() {
        assert!(endpoint_is_orbstack("unix:///Users/u/.orbstack/run/docker.sock"));
        assert!(endpoint_is_orbstack("tcp://orbstack.internal:2375"));
        assert!(!endpoint_is_orbstack("unix:///var/run/docker.sock"));
        assert!(!endpoint_is_orbstack("npipe:////./pipe/docker_engine"));
    }

    #[test]
    fn status_probe_by_makefile_and_config() {
        let dir = std::env::temp_dir().join(format!("forge-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(status(&dir), Status::Empty);
        std::fs::write(dir.join("Makefile"), "").unwrap();
        assert_eq!(status(&dir), Status::Cloned);
        std::fs::write(dir.join(".config"), "").unwrap();
        assert_eq!(status(&dir), Status::Configured);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
