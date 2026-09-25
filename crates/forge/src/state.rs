//! 活动卷状态（current）：`.virtuoso/kernel-current.json`。
//!
//! current 不进 `virtuoso.toml`（唯一配置面留给持久启动配置），用专门的
//! 文件表示"当前是哪个卷"——机器本地会话状态，git 忽略；`kernel use/clone`
//! 写入，`path/list` 与 doctor 读它。写 current 的唯一入口 [`write`] 同步
//! 渲染 `.devcontainer/devcontainer.json`（devcontainer 跟随活动卷，见
//! `devcontainer` 模块）。

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// 当前活动卷（volume + 目标架构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Current {
    pub volume: String,
    pub arch: String,
}

/// 状态文件路径（repo 根 `.virtuoso/kernel-current.json`）。
pub fn path(project_root: &Path) -> PathBuf {
    project_root.join(".virtuoso").join("kernel-current.json")
}

/// 缺失 = 无活动卷（raw/preset 模式，doctor 零打扰）；存在但非法 = 报错。
pub fn read(project_root: &Path) -> anyhow::Result<Option<Current>> {
    let p = path(project_root);
    if !p.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p)
        .with_context(|| format!("读取 {}", p.display()))?;
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| {
            format!(
                "{} 损坏（期望 {{volume, arch}}）；重新执行 `virtuoso kernel use <volume>` 修复",
                p.display()
            )
        })
}

/// 写入（目录不存在则创建；父目录 `.virtuoso/` 已 git 忽略）。
/// 同步渲染 `.devcontainer/devcontainer.json`——current 变更必须跟随，
/// 放唯一写入口免得未来新增 writer 漏挂。
pub fn write(project_root: &Path, current: &Current) -> anyhow::Result<()> {
    let p = path(project_root);
    std::fs::create_dir_all(p.parent().expect("状态文件必有父目录"))?;
    std::fs::write(&p, format!("{}\n", serde_json::to_string_pretty(current)?))?;
    crate::devcontainer::render(project_root, current)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试独立 scratch（并行测试共用 pid 名会互相踩踏）。
    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("forge-state-{name}-{}", std::process::id()))
    }

    #[test]
    fn missing_state_file_reads_as_none() {
        let dir = scratch("missing");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read(&dir).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_read_roundtrip() {
        let dir = scratch("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let cur = Current {
            volume: "ksrc-oe66".into(),
            arch: "arm64".into(),
        };
        write(&dir, &cur).unwrap();
        assert_eq!(read(&dir).unwrap(), Some(cur));
        assert!(dir.join(".virtuoso").join("kernel-current.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_state_file_is_an_error_with_hint() {
        let dir = scratch("corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".virtuoso")).unwrap();
        std::fs::write(path(&dir), "{ not json").unwrap();
        let err = format!("{}", read(&dir).unwrap_err());
        assert!(err.contains("kernel use"), "hint missing: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
