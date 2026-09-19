//! 类型化配置层。Phase 2：`.env` 全量兼容 + 可选 `virtuoso.toml`（推荐，
//! 覆盖 .env 同名键）；非法配置在解析期报错（如 SMP 不被 NUMA_NODES 整除）。
//!
//! 兼容性约定（与 qemu-e2e 的 Makefile `-include .env` 及脚本内
//! `set -a; . .env` 语义一致）：**.env 中的值优先于进程环境变量**；
//! `virtuoso.toml` 优先于 .env。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub use launcher::Arch;

/// `.env` 解析（KEY=VALUE / `export KEY=VALUE`，支持 `#` 注释与成对引号）。
/// 可选 `virtuoso.toml` 覆盖同名键（Phase 2 推荐配置面，强类型校验）。
#[derive(Debug, Default)]
pub struct EnvFile {
    pub vars: BTreeMap<String, String>,
    pub path: Option<PathBuf>,
    pub toml_path: Option<PathBuf>,
}

impl EnvFile {
    pub fn load(project_root: &Path) -> anyhow::Result<Self> {
        let path = project_root.join(".env");
        let mut vars = BTreeMap::new();
        if path.is_file() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 {} 失败", path.display()))?;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let k = k.trim();
                if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                let v = v.trim();
                // 成对引号剥除（内联注释的展开交给脚本，这里保持原样以免破坏 QEMU_OPTS）
                let v = if v.len() >= 2
                    && ((v.starts_with('"') && v.ends_with('"'))
                        || (v.starts_with('\'') && v.ends_with('\'')))
                {
                    &v[1..v.len() - 1]
                } else {
                    v
                };
                vars.insert(k.to_string(), v.to_string());
            }
        }
        let path = path.is_file().then_some(path);

        // ---- virtuoso.toml 覆盖层（存在的键优先于 .env）----
        let toml_path = project_root.join("virtuoso.toml");
        let toml_path = if toml_path.is_file() {
            apply_toml(&mut vars, &toml_path)?;
            Some(toml_path)
        } else {
            None
        };

        Ok(Self { vars, path, toml_path })
    }

    /// BusyBox 供给配置（BUSYBOX_* 变量 → builder::busybox::Supply）。
    pub fn busybox_supply(&self) -> builder::busybox::Supply {
        builder::busybox::Supply {
            version: self.get("BUSYBOX_VERSION").filter(|s| !s.is_empty()),
            release_repo: self.get("BUSYBOX_RELEASE_REPO").filter(|s| !s.is_empty()),
            dl_url: self.get("BUSYBOX_DL_URL").filter(|s| !s.is_empty()),
            force_source_build: self.get("BUSYBOX_SOURCE_BUILD").as_deref() == Some("1"),
        }
    }

    /// .env 优先，其次进程环境变量（与 Makefile/脚本行为对齐）。
    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(v) = self.vars.get(key) {
            return Some(v.clone());
        }
        std::env::var(key).ok()
    }
}

/// 项目根定位：从当前目录逐级向上寻找含 `infra/verify.sh` 的目录。
pub fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("infra/verify.sh").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub struct Config {
    pub project_root: PathBuf,
    pub infra_dir: PathBuf,
    pub env: EnvFile,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let project_root = find_project_root().ok_or_else(|| {
            anyhow::anyhow!(
                "virtuoso 项目根未找到（需要 infra/verify.sh）；请在 qemu-e2e 检出目录内运行"
            )
        })?;
        let infra_dir = project_root.join("infra");
        let env = EnvFile::load(&project_root)?;
        Ok(Self {
            project_root,
            infra_dir,
            env,
        })
    }

    /// BusyBox 供给配置（透传 env 层）。
    pub fn busybox_supply(&self) -> builder::busybox::Supply {
        self.env.busybox_supply()
    }

    /// 解析目标架构；无法解析时返回 None（诊断层告警，具体行为仍由脚本裁决以保持 parity）。
    pub fn arch(&self) -> Option<Arch> {
        self.env
            .get("ARCH")
            .and_then(|a| Arch::parse(&a))
            .or_else(Arch::host_default)
    }

    /// (KERNEL_PATH, 是否显式指定)。未指定时 = 项目根上一级（qemu-e2e 自动探测语义）。
    pub fn kernel_path(&self) -> anyhow::Result<(PathBuf, bool)> {
        match self.env.get("KERNEL_PATH").filter(|s| !s.is_empty()) {
            Some(p) => Ok((PathBuf::from(p), true)),
            None => {
                let parent = self
                    .project_root
                    .parent()
                    .context("项目根没有上级目录，无法自动探测 KERNEL_PATH")?;
                Ok((parent.to_path_buf(), false))
            }
        }
    }

    /// QEMU_TIMEOUT 原始字符串（保持与 make 透传 timeout 一致；"0" 由 test 命令拒绝）。
    pub fn timeout_raw(&self) -> String {
        self.env
            .get("QEMU_TIMEOUT")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "0".into())
    }

    pub fn auto_test(&self) -> String {
        self.env
            .get("AUTO_TEST")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "1".into())
    }

}

/// 接受 string 或 integer 标量并统一成 String（timeout_secs = 60 与 = "60"
/// 等价，兼容 Makefile 透传语义）。其余类型（bool/array…）在解析期报错。
struct StrVal(String);

impl<'de> serde::Deserialize<'de> for StrVal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = StrVal;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("字符串或整数")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<StrVal, E> {
                Ok(StrVal(v.to_string()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<StrVal, E> {
                Ok(StrVal(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<StrVal, E> {
                Ok(StrVal(v.to_string()))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlOverlay {
    kernel_path: Option<StrVal>,
    arch: Option<StrVal>,
    timeout_secs: Option<StrVal>,
    smp: Option<StrVal>,
    qemu: Option<StrVal>,
    backend: Option<StrVal>,
    qemu_opts: Option<Vec<String>>,
    numa: Option<NumaSection>,
    busybox: Option<BusyboxSection>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NumaSection {
    memory_per_node: Option<StrVal>,
    nodes: Option<StrVal>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BusyboxSection {
    version: Option<StrVal>,
    release_repo: Option<StrVal>,
    dl_url: Option<StrVal>,
    force_source_build: Option<bool>,
}

impl StrVal {
    fn into_string(self) -> String {
        self.0
    }
}

/// virtuoso.toml → env 键映射（存在的键覆盖 .env）。schema 用 serde 结构体
/// 表达：未知键与非法类型一律在解析期报错（Phase 2 的强类型目标之一）。
fn apply_toml(vars: &mut BTreeMap<String, String>, path: &Path) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path.display()))?;
    let cfg: TomlOverlay = toml::from_str(&raw)
        .with_context(|| format!("{} 解析失败（未知键或非法类型）", path.display()))?;

    fn put(vars: &mut BTreeMap<String, String>, key: &str, v: Option<StrVal>) {
        if let Some(v) = v {
            vars.insert(key.to_string(), v.into_string());
        }
    }

    put(vars, "KERNEL_PATH", cfg.kernel_path);
    put(vars, "ARCH", cfg.arch);
    put(vars, "QEMU_TIMEOUT", cfg.timeout_secs);
    put(vars, "SMP", cfg.smp);
    put(vars, "QEMU", cfg.qemu);
    put(vars, "BACKEND", cfg.backend);
    if let Some(list) = cfg.qemu_opts {
        if !list.is_empty() {
            vars.insert("QEMU_OPTS".into(), list.join(" "));
        }
    }
    if let Some(numa) = cfg.numa {
        put(vars, "NUMA_MEMORY", numa.memory_per_node);
        put(vars, "NUMA_NODES", numa.nodes);
    }
    if let Some(bb) = cfg.busybox {
        put(vars, "BUSYBOX_VERSION", bb.version);
        put(vars, "BUSYBOX_RELEASE_REPO", bb.release_repo);
        put(vars, "BUSYBOX_DL_URL", bb.dl_url);
        if bb.force_source_build == Some(true) {
            vars.insert("BUSYBOX_SOURCE_BUILD".into(), "1".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_overlay_applies_to_env_vars() {
        let dir = std::env::temp_dir().join(format!("virtuoso-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("virtuoso.toml");
        std::fs::write(
            &path,
            r#"
arch = "arm64"
timeout_secs = 60
smp = 4
qemu_opts = ["-device vfio-pci,host=01:00.0"]
[numa]
nodes = 2
memory_per_node = "1G"
[busybox]
version = "1.36.1"
force_source_build = true
"#,
        )
        .unwrap();
        let mut vars = BTreeMap::new();
        apply_toml(&mut vars, &path).unwrap();
        assert_eq!(vars.get("ARCH").map(String::as_str), Some("arm64"));
        assert_eq!(vars.get("QEMU_TIMEOUT").map(String::as_str), Some("60"));
        assert_eq!(vars.get("SMP").map(String::as_str), Some("4"));
        assert_eq!(
            vars.get("QEMU_OPTS").map(String::as_str),
            Some("-device vfio-pci,host=01:00.0")
        );
        assert_eq!(vars.get("NUMA_NODES").map(String::as_str), Some("2"));
        assert_eq!(vars.get("BUSYBOX_VERSION").map(String::as_str), Some("1.36.1"));
        assert_eq!(vars.get("BUSYBOX_SOURCE_BUILD").map(String::as_str), Some("1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_key_is_rejected_at_parse_time() {
        let cfg: Result<TomlOverlay, _> = toml::from_str("arch = \"arm64\"\nshmp = 4\n");
        assert!(cfg.is_err(), "拼写错误的键必须在解析期报错");
    }

    #[test]
    fn wrong_type_is_rejected_at_parse_time() {
        assert!(toml::from_str::<TomlOverlay>("qemu_opts = [1, 2]").is_err());
        assert!(toml::from_str::<TomlOverlay>("arch = true").is_err());
    }
}
