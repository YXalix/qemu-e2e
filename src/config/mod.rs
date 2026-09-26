//! 类型化配置层。`virtuoso.toml` 是唯一配置面 —— 结构化全局键 +
//! 组件化（每个 `[components.*]` 段是一个 VM 组件，`require` 声明其依赖的
//! 内核模块，builder 按启用组件的并集生成模块清单；`[tests]` 段收测例
//! 套件的模块依赖）。非法配置一律解析期报错（未知键、非法类型）。
//!
//! 标量键取值优先级：**进程环境变量 > virtuoso.toml** —— 同名键环境变量
//! 覆盖 toml 字段（CI/命令行临时改参不动文件），都未设置时用内置缺省。
//!
//! 模块划分（按关注点解耦）：
//! - `schema`   —— toml schema 与解析（未知键/非法类型解析期报错的唯一防线）
//! - `global`   —— 全局标量访问器（arch/timeout/kernel*/qemu 透传）
//! - `components` —— 组件开关与访问器（tools_disk/agent/vfio/numa/pmem）
//! - `plan`     —— ComponentPlan：启用组件 require 并集的 stage 分区
//! - `busybox`  —— BusyBox 供给配置

mod busybox;
mod components;
mod global;
mod plan;
mod schema;

use std::path::{Path, PathBuf};

pub use plan::ComponentPlan;
pub use schema::VirtuosoToml;

/// 项目根定位：从当前目录逐级向上寻找含 `infra/init`（PID 1 脚本）的目录。
pub fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("infra/init").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// 标量取值链：进程环境变量 > virtuoso.toml 字段（同名键 env 覆盖 toml；
/// 空串视为未设置，继续回落）。无文件编辑的临时改参走 env，持久配置写 toml。
fn scalar(toml_val: Option<&schema::StrVal>, env_key: &str) -> Option<String> {
    if let Ok(v) = std::env::var(env_key) {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    if let Some(v) = toml_val {
        if !v.0.trim().is_empty() {
            return Some(v.0.clone());
        }
    }
    None
}

/// env 布尔解析（宽松语义："1"/"true"/"yes"/"on" 为真，其余为假）。
fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub struct Config {
    pub project_root: PathBuf,
    /// VM 内源资产（init、modules-boot.conf、testcases、tools）——git 跟踪。
    pub infra_dir: PathBuf,
    /// 工作区 target/（运行 scratch：agent shell socket 等）。
    pub target_dir: PathBuf,
    /// 构建产物（initrd.img / rootfs.img / tools.img）——数据面，git 忽略。
    pub artifacts_dir: PathBuf,
    /// 构建暂存与缓存（busybox 供给链、initramfs/rootfs 组装目录）——git 忽略。
    pub build_dir: PathBuf,
    /// virtuoso.toml 路径（存在才 Some）。
    pub toml_path: Option<PathBuf>,
    /// virtuoso.toml 类型化解析结果（文件存在才 Some；未知键/非法类型已在
    /// 解析期报错）。
    pub toml: Option<VirtuosoToml>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let project_root = find_project_root().ok_or_else(|| {
            anyhow::anyhow!(
                "virtuoso project root not found (needs infra/init); run inside a virtuoso checkout"
            )
        })?;
        let infra_dir = project_root.join("infra");
        let target_dir = project_root.join("target");
        let toml_path = project_root.join("virtuoso.toml");
        let toml = if toml_path.is_file() {
            Some(schema::parse_toml(&toml_path)?)
        } else {
            None
        };
        Ok(Self {
            artifacts_dir: target_dir.join("artifacts"),
            build_dir: target_dir.join("build"),
            project_root,
            infra_dir,
            target_dir,
            toml_path: toml_path.is_file().then_some(toml_path),
            toml,
        })
    }

    /// virtuoso.toml 路径（存在才 Some）。
    pub fn toml_path(&self) -> Option<&Path> {
        self.toml_path.as_deref()
    }

    /// 取 toml 全局标量字段（None 当字段未写或 toml 文件不存在）。
    fn tv(&self, f: impl FnOnce(&VirtuosoToml) -> &Option<schema::StrVal>) -> Option<&schema::StrVal> {
        self.toml.as_ref().and_then(|t| f(t).as_ref())
    }

    /// 组件段访问入口（toml 未配置 / components 段缺失 = None）。
    fn comp<'a, T>(
        &'a self,
        f: impl FnOnce(&'a schema::ComponentsSection) -> Option<&'a T>,
    ) -> Option<&'a T> {
        f(self.toml.as_ref()?.components.as_ref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_env_overrides_toml() {
        // 优先级冻结：进程环境变量 > virtuoso.toml。env 改动是进程全局的，
        // 用互斥锁串行化，键名用专用前缀避免污染真实配置键。
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();
        let toml = schema::StrVal("arm64".into());
        std::env::set_var("VIRTUOSO_TEST_SCALAR", "riscv64");
        assert_eq!(
            scalar(Some(&toml), "VIRTUOSO_TEST_SCALAR").as_deref(),
            Some("riscv64"),
            "同名键进程环境变量必须覆盖 toml"
        );
        std::env::remove_var("VIRTUOSO_TEST_SCALAR");
        assert_eq!(
            scalar(Some(&toml), "VIRTUOSO_TEST_SCALAR").as_deref(),
            Some("arm64"),
            "env 未设置时回落 toml"
        );
        // 空串视为未设置：env 与 toml 的空值都继续回落
        std::env::set_var("VIRTUOSO_TEST_SCALAR", "");
        assert_eq!(
            scalar(Some(&toml), "VIRTUOSO_TEST_SCALAR").as_deref(),
            Some("arm64")
        );
        std::env::remove_var("VIRTUOSO_TEST_SCALAR");
        assert_eq!(
            scalar(Some(&schema::StrVal("  ".into())), "VIRTUOSO_TEST_SCALAR"),
            None
        );
        assert_eq!(scalar(None, "VIRTUOSO_TEST_SCALAR"), None);
    }
}
