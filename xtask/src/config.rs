//! 类型化配置层（Phase 1：诊断用；Phase 2 起接管运行时配置）。
//!
//! 兼容性约定（与 qemu-e2e 的 Makefile `-include .env` 及脚本内
//! `set -a; . .env` 语义一致）：**.env 中的值优先于进程环境变量**。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// 目标架构矩阵 —— qemu-e2e 中散落在 3 个 shell 脚本里的表格的唯一事实来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X86_64,
    Riscv64,
}

impl Arch {
    /// 与 verify.sh / run-qemu.sh 相同的别名归一逻辑。
    pub fn parse(s: &str) -> Option<Arch> {
        match s {
            "aarch64" | "arm64" => Some(Arch::Arm64),
            "x86_64" | "amd64" => Some(Arch::X86_64),
            "riscv64" => Some(Arch::Riscv64),
            _ => None,
        }
    }

    /// 宿主机架构（脚本中的 `uname -m` 等价物）。
    pub fn host_default() -> Option<Arch> {
        Self::parse(std::env::consts::ARCH)
    }

    pub fn name(self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X86_64 => "x86_64",
            Arch::Riscv64 => "riscv64",
        }
    }

    pub fn qemu_bin(self) -> &'static str {
        match self {
            Arch::Arm64 => "qemu-system-aarch64",
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::Riscv64 => "qemu-system-riscv64",
        }
    }

    /// 相对内核树的镜像路径。
    pub fn kernel_img(self) -> &'static str {
        match self {
            Arch::Arm64 => "arch/arm64/boot/Image",
            Arch::X86_64 => "arch/x86/boot/bzImage",
            Arch::Riscv64 => "arch/riscv/boot/Image",
        }
    }

    pub fn console(self) -> &'static str {
        match self {
            Arch::Arm64 => "ttyAMA0",
            Arch::X86_64 | Arch::Riscv64 => "ttyS0",
        }
    }

    pub fn machine(self) -> &'static str {
        match self {
            Arch::X86_64 => "q35",
            _ => "virt",
        }
    }
}

/// `.env` 解析（KEY=VALUE / `export KEY=VALUE`，支持 `#` 注释与成对引号）。
#[derive(Debug, Default)]
pub struct EnvFile {
    pub vars: BTreeMap<String, String>,
    pub path: Option<PathBuf>,
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
        Ok(Self { vars, path })
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

    /// Phase 1 配置诊断（cargo xtask verify 前置输出）。
    pub fn print_diagnostics(&self, arch_override: Option<&str>) {
        println!("[CONFIG] Virtuoso Phase-1 wrapper — typed config diagnostics");
        println!(
            "  .env: {}",
            self.env
                .path
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "absent (defaults; cp .env.example .env)".into())
        );

        // 架构
        let arch_raw = arch_override
            .map(str::to_string)
            .or_else(|| self.env.get("ARCH"));
        let host = Arch::host_default();
        match arch_raw.as_deref().and_then(Arch::parse).or(host) {
            Some(arch) => {
                let src = if arch_override.is_some() {
                    "CLI --arch"
                } else if self.env.get("ARCH").is_some() {
                    "config"
                } else {
                    "host default"
                };
                println!(
                    "  arch: {} ({src}; qemu={}, machine={}, console={})",
                    arch.name(),
                    arch.qemu_bin(),
                    arch.machine(),
                    arch.console()
                );
                if host.is_some_and(|h| h != arch) {
                    println!(
                        "  WARN: cross-compile ARCH={} differs from host ({}), ensure cross-toolchain is available",
                        arch.name(),
                        std::env::consts::ARCH
                    );
                }
            }
            None => {
                if let Some(raw) = arch_raw {
                    println!("  ERROR: Unsupported ARCH={raw}");
                }
            }
        }

        // 内核路径与镜像
        match self.kernel_path() {
            Ok((kp, explicit)) => {
                println!(
                    "  kernel_path: {} ({})",
                    kp.display(),
                    if explicit { "from config" } else { "auto-detected" }
                );
                if let Some(arch) = self.arch() {
                    let img = kp.join(arch.kernel_img());
                    println!(
                        "  kernel_image: {} — {}",
                        arch.kernel_img(),
                        if img.is_file() { "found" } else { "MISSING (build the kernel first)" }
                    );
                }
            }
            Err(e) => println!("  ERROR: {e}"),
        }

        // CPU / NUMA
        let smp = self.env.get("SMP").unwrap_or_else(|| "8".into());
        let nodes_raw = self.env.get("NUMA_NODES").unwrap_or_else(|| "1".into());
        let mem = self.env.get("NUMA_MEMORY").unwrap_or_else(|| "1G".into());
        let (smp_n, nodes_n) = (smp.trim().parse::<u32>().ok(), nodes_raw.trim().parse::<u32>().ok());
        match (smp_n, nodes_n) {
            (Some(s), Some(n)) => {
                let total = total_memory(&mem, n)
                    .unwrap_or_else(|| format!("{mem} (unparsed)"));
                println!("  cpu/mem: smp={s}, numa_nodes={n}, per-node={mem}, total={total}");
                if n > 1 && s % n != 0 {
                    println!(
                        "  WARN: SMP ({s}) not divisible by NUMA_NODES ({n}) — run-qemu.sh will reject"
                    );
                }
            }
            _ => println!("  WARN: SMP/NUMA_NODES 非数字（{smp} / {nodes_raw}），交由脚本裁决"),
        }

        // 超时
        let t = self.timeout_raw();
        println!(
            "  timeout: {t}s{}",
            if t == "0" { " (cargo xtask test will reject 0)" } else { "" }
        );

        // QEMU 二进制
        if let Some(arch) = self.arch() {
            let override_q = self.env.get("QEMU");
            let found = match &override_q {
                Some(q) => which_exists(q),
                None => which_exists(arch.qemu_bin()),
            };
            let label = override_q.as_deref().unwrap_or(arch.qemu_bin());
            println!(
                "  qemu: {label} — {}",
                if found { "found" } else { "NOT FOUND (install qemu-system or set QEMU=)" }
            );
        }
        println!();
    }
}

fn total_memory(mem: &str, nodes: u32) -> Option<String> {
    let digits = mem.find(|c: char| !c.is_ascii_digit()).unwrap_or(mem.len());
    let n: u64 = mem[..digits].parse().ok()?;
    let suffix = mem[digits..].trim();
    let total = n.saturating_mul(nodes as u64);
    match suffix.to_ascii_uppercase().as_str() {
        "G" => Some(format!("{total}G")),
        "M" => Some(format!("{total}M")),
        "" => Some(format!("{total}")),
        _ => None,
    }
}

fn which_exists(bin: &str) -> bool {
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file())
        })
        .unwrap_or(false)
}
