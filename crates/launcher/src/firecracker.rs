//! firecracker — Phase 3 第二后端：microVM 池（boot 尾延迟敏感场景）。
//!
//! 与 QEMU 后端的关系：**判定协议不变**（标记协议 v1 走 firecracker 进程的
//! stdout，init/judge 一概不感知后端差异）；`spawn` 返回值与 guardian 收割
//! 约定一致（独立进程组）。
//!
//! 硬约束（preflight 逐项给出可操作诊断）：
//! - 仅 x86_64 / aarch64（Firecracker 上游不支持 riscv64）；
//! - KVM 必需（/dev/kvm）——Firecracker 无 TCG 等价物；
//! - 无 initramfs 支持：两段式引导退化为单段，内核必须把 virtio-blk 与
//!   ext4 **内建**（=y）。openEuler 缺省配置是 =m（见 docs 的怪癖表），
//!   因此 preflight 会读内核 .config 逐项核对；
//! - aarch64 内核必须交付 ELF（vmlinux），`Image` 裸镜像不被接受。

use anyhow::{bail, Context};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::{Arch, NumaTopology};

/// Firecracker 支持的架构。
pub fn arch_supported(arch: Arch) -> bool {
    matches!(arch, Arch::Arm64 | Arch::X86_64)
}

/// boot_args：与 QEMU 后端 cmdline 同源（console 由架构决定；auto_test 开关
/// 语义一致）。`reboot=k` 使 panic 行为对齐 `-no-reboot` 的判定假设。
pub fn boot_args(arch: Arch, auto_test: bool) -> String {
    let mut s = format!(
        "console={} root=/dev/vda rw init=/init loglevel=8 reboot=k panic=1 pci=off",
        arch.console()
    );
    if auto_test {
        s.push_str(" auto_test");
    }
    s
}

/// Firecracker config-file JSON（单文件启动形态；API 逐 PUT 形态见
/// `api_requests`）。字段名是 Firecracker v1 API 的冻结面。
#[derive(Debug, Clone)]
pub struct FirecrackerInvocation {
    pub arch: Arch,
    /// ELF 内核（aarch64）/ bzImage（x86_64）
    pub kernel: PathBuf,
    /// ext4 rootfs（直接作根，无 initramfs）
    pub rootfs: PathBuf,
    pub vcpus: u32,
    /// MiB
    pub mem_mib: u64,
    pub auto_test: bool,
    /// firecracker 可执行文件路径（缺省 PATH 查找 "firecracker"）
    pub binary: Option<PathBuf>,
    /// config JSON 落盘路径（run 目录内，作为运行工件）
    pub config_path: PathBuf,
    /// unix api socket 路径（--api-sock）
    pub api_sock: PathBuf,
}

/// "2G"/"512M"/裸数字 → MiB（与 NumaTopology::total_memory 的单位语义一致，
/// 解析统一走 common::units）。
pub fn memory_to_mib(total: &str) -> Result<u64, String> {
    let (n, unit) = common::units::parse_memory(total)?;
    Ok(unit.to_mib(n))
}

impl FirecrackerInvocation {
    pub fn new(
        arch: Arch,
        kernel: impl Into<PathBuf>,
        rootfs: impl Into<PathBuf>,
        topo: &NumaTopology,
        auto_test: bool,
        config_path: impl Into<PathBuf>,
        api_sock: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        if !arch_supported(arch) {
            return Err(format!(
                "firecracker 不支持架构 {}（仅 x86_64/aarch64）",
                arch.name()
            ));
        }
        let mem_mib = memory_to_mib(&topo.total_memory()?)?;
        Ok(Self {
            arch,
            kernel: kernel.into(),
            rootfs: rootfs.into(),
            vcpus: topo.smp,
            mem_mib,
            auto_test,
            binary: None,
            config_path: config_path.into(),
            api_sock: api_sock.into(),
        })
    }

    pub fn binary(&self) -> &Path {
        match &self.binary {
            Some(p) => p.as_path(),
            None => Path::new("firecracker"),
        }
    }

    /// 单行可复制启动命令（shell 引用；spawn 前展示 / 手动复现用）。
    /// 复现前需确保 config_path 的 JSON 已存在（spawn 会自动写入）。
    pub fn command_line(&self) -> String {
        format!(
            "{} --api-sock {} --config-file {}",
            self.binary().display(),
            crate::shell_quote(&self.api_sock.display().to_string()),
            crate::shell_quote(&self.config_path.display().to_string()),
        )
    }

    /// config-file JSON 文本（serde_json 手写以保证键序稳定可 diff）。
    pub fn config_json(&self) -> String {
        let mut s = String::from("{\n");
        s.push_str(&format!(
            "  \"boot-source\": {{\"kernel_image_path\": \"{}\", \"boot_args\": \"{}\"}},\n",
            self.kernel.display(),
            boot_args(self.arch, self.auto_test)
        ));
        s.push_str(&format!(
            "  \"machine-config\": {{\"vcpu_count\": {}, \"mem_size_mib\": {}, \"smt\": false}},\n",
            self.vcpus, self.mem_mib
        ));
        s.push_str(&format!(
            "  \"drives\": [{{\"drive_id\": \"rootfs\", \"path_on_host\": \"{}\", \"is_root_device\": true, \"is_read_only\": false}}]\n",
            self.rootfs.display()
        ));
        s.push('}');
        s
    }

    /// 等价的 API 逐 PUT 序列（--api-sock 交互形态，供 curl/jq 调试与文档）。
    pub fn api_requests(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PUT /boot-source",
                format!(
                    "{{\"kernel_image_path\": \"{}\", \"boot_args\": \"{}\"}}",
                    self.kernel.display(),
                    boot_args(self.arch, self.auto_test)
                ),
            ),
            (
                "PUT /machine-config",
                format!("{{\"vcpu_count\": {}, \"mem_size_mib\": {}}}", self.vcpus, self.mem_mib),
            ),
            (
                "PUT /drives/rootfs",
                format!(
                    "{{\"drive_id\": \"rootfs\", \"path_on_host\": \"{}\", \"is_root_device\": true, \"is_read_only\": false}}",
                    self.rootfs.display()
                ),
            ),
            ("PUT /actions", "{\"action_type\": \"InstanceStart\"}".to_string()),
        ]
    }

    pub fn write_config(&self) -> anyhow::Result<()> {
        std::fs::write(&self.config_path, self.config_json())
            .with_context(|| format!("写 firecracker 配置 {} 失败", self.config_path.display()))
    }

    /// spawn：独立进程组（pgid = child pid，与 QemuInvocation 同约定，交给
    /// guardian 收割）。guest 串口接 firecracker 进程 stdout，piped 语义与 QEMU 一致。
    /// spawn：独立进程组（pgid = child pid，与 QemuInvocation 同约定，交给
    /// guardian 收割）。guest 串口接 firecracker 进程 stdout，piped 语义与 QEMU 一致。
    pub fn spawn(&self, piped: bool) -> anyhow::Result<(Child, u32)> {
        self.write_config()?;
        let mut cmd = Command::new(self.binary());
        cmd.arg("--api-sock")
            .arg(&self.api_sock)
            .arg("--config-file")
            .arg(&self.config_path)
            .process_group(0);
        if piped {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null());
        }
        let child = cmd.spawn().with_context(|| {
            format!(
                "{} 启动失败（preflight 见 `cargo xtask verify --backend firecracker`）",
                self.binary().display()
            )
        })?;
        let pgid = child.id();
        Ok((child, pgid))
    }

    /// spawn 并登记监管：注册表 + 收割守卫一步到位（与 QemuInvocation 同约定）。
    pub fn spawn_supervised(&self, piped: bool) -> anyhow::Result<(Child, guardian::Supervised)> {
        let (child, pgid) = self.spawn(piped)?;
        Ok((child, guardian::Supervised::adopt(pgid)))
    }
}

/// preflight 单项结论（`cargo xtask verify` 的诊断视图）。
pub struct PreflightCheck {
    pub name: &'static str,
    pub ok: bool,
    pub note: String,
}

/// microVM 引导硬前提逐项核对（诊断视图：只收集结论，不做硬失败）。
/// `kernel_config` 为 None（内核树未解析）时内建项记为未通过。
pub fn preflight_checks(
    arch: Arch,
    kernel: &Path,
    kernel_config: Option<&Path>,
    binary_name: &str,
) -> Vec<PreflightCheck> {
    vec![
        PreflightCheck {
            name: "arch supported",
            ok: arch_supported(arch),
            note: "仅 x86_64/aarch64".into(),
        },
        PreflightCheck {
            name: "/dev/kvm",
            ok: Path::new("/dev/kvm").exists(),
            note: "KVM 必需（无 TCG 等价物）".into(),
        },
        PreflightCheck {
            name: "firecracker binary",
            ok: common::fsutil::which(binary_name),
            note: "FIRECRACKER_BIN 可覆盖".into(),
        },
        PreflightCheck {
            name: "kernel format",
            ok: kernel_format_ok(arch, kernel).is_ok(),
            note: format!("{}（aarch64 要求 ELF vmlinux）", kernel.display()),
        },
        PreflightCheck {
            name: "builtin virtio/ext4/serial",
            ok: kernel_config
                .and_then(|kc| required_builtin_missing(arch, kc).ok())
                .map(|m| m.is_empty())
                .unwrap_or(false),
            note: ".config 内建 =y（无 initramfs 引导）".into(),
        },
    ]
}

/// spawn 前硬校验：逐项核对，首个失败即 Err（可操作诊断）。
/// microVM 引导的硬前提，见本模块文档（`cargo xtask verify --backend firecracker`
/// 提供同样的逐项诊断视图）。
pub fn preflight(
    arch: Arch,
    kernel: &Path,
    kernel_config: &Path,
    binary_name: &str,
) -> anyhow::Result<()> {
    if !arch_supported(arch) {
        anyhow::bail!(
            "firecracker 不支持架构 {}（仅 x86_64/aarch64）",
            arch.name()
        );
    }
    if !Path::new("/dev/kvm").exists() {
        anyhow::bail!("firecracker 需要 KVM：/dev/kvm 不存在（加载 kvm 模块或改用 qemu 后端）");
    }
    if !common::fsutil::which(binary_name) {
        anyhow::bail!(
            "firecracker 二进制未找到（{binary_name}）——安装后重试，或用 FIRECRACKER_BIN 指定路径"
        );
    }
    kernel_format_ok(arch, kernel)?;
    let missing = required_builtin_missing(arch, kernel_config)?;
    if !missing.is_empty() {
        anyhow::bail!(
            "firecracker 无 initramfs 引导，以下内核配置必须内建（=y）：{}\n  在内核树 make menuconfig 打开后重新编译",
            missing.join(", ")
        );
    }
    Ok(())
}

/// aarch64 内核必须是 ELF（vmlinux）；x86_64 bzImage 无此要求。
pub fn kernel_format_ok(arch: Arch, kernel: &Path) -> anyhow::Result<()> {
    if arch != Arch::Arm64 {
        return Ok(());
    }
    let magic = common::fsutil::read_magic(kernel)
        .with_context(|| format!("读取内核 {} 失败", kernel.display()))?;
    if &magic != b"\x7fELF" {
        bail!(
            "firecracker(aarch64) 要求 ELF 内核（vmlinux），{} 不是 ELF —— 改用 arch/arm64/boot/compressed/vmlinux 或 make vmlinux 产物",
            kernel.display()
        );
    }
    Ok(())
}

/// 内核 .config 内建项核对（无 initramfs 引导的硬前提）。
/// 返回缺失列表；空列表 = 可引导。config 不存在时返回 Err（无法背书）。
/// 串口 console 内建项按架构区分（aarch64 = PL011，x86_64 = 8250）。
pub fn required_builtin_missing(
    arch: Arch,
    kernel_config: &Path,
) -> anyhow::Result<Vec<&'static str>> {
    let text = std::fs::read_to_string(kernel_config)
        .with_context(|| format!("读取 {} 失败（内核树未配置？）", kernel_config.display()))?;
    let mut need: Vec<&str> = vec!["CONFIG_VIRTIO", "CONFIG_VIRTIO_BLK", "CONFIG_EXT4_FS"];
    match arch {
        Arch::Arm64 => need.push("CONFIG_SERIAL_AMBA_PL011"),
        Arch::X86_64 => need.push("CONFIG_SERIAL_8250"),
        Arch::Riscv64 => {}
    }
    let mut missing = Vec::new();
    for key in need {
        let Some(line) = text.lines().find(|l| l.starts_with(key)) else {
            missing.push(key);
            continue;
        };
        if !line.ends_with("=y") {
            missing.push(key);
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topo() -> NumaTopology {
        NumaTopology::parse("4", "1", "1G").unwrap()
    }

    fn inv() -> FirecrackerInvocation {
        FirecrackerInvocation::new(
            Arch::Arm64,
            "/tmp/vmlinux",
            "/tmp/rootfs.img",
            &topo(),
            true,
            "/tmp/fc-config.json",
            "/tmp/fc.sock",
        )
        .unwrap()
    }

    #[test]
    fn riscv64_rejected() {
        assert!(FirecrackerInvocation::new(
            Arch::Riscv64,
            "k",
            "r",
            &topo(),
            false,
            "/tmp/c",
            "/tmp/s"
        )
        .is_err());
    }

    #[test]
    fn config_json_contains_frozen_v1_fields() {
        let j = inv().config_json();
        assert!(j.contains("\"kernel_image_path\": \"/tmp/vmlinux\""));
        assert!(j.contains("\"boot_args\": \"console=ttyAMA0 root=/dev/vda rw init=/init loglevel=8 reboot=k panic=1 pci=off auto_test\""));
        assert!(j.contains("\"vcpu_count\": 4"));
        assert!(j.contains("\"mem_size_mib\": 1024"));
        assert!(j.contains("\"is_root_device\": true"));
        assert!(j.contains("\"path_on_host\": \"/tmp/rootfs.img\""));
    }

    #[test]
    fn api_requests_cover_boot_machine_drive_start() {
        let reqs = inv().api_requests();
        let names: Vec<_> = reqs.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "PUT /boot-source",
                "PUT /machine-config",
                "PUT /drives/rootfs",
                "PUT /actions"
            ]
        );
    }

    #[test]
    fn memory_units_to_mib() {
        assert_eq!(memory_to_mib("2G").unwrap(), 2048);
        assert_eq!(memory_to_mib("512M").unwrap(), 512);
        assert_eq!(memory_to_mib("1024").unwrap(), 1024);
        assert!(memory_to_mib("1X").is_err());
    }

    #[test]
    fn boot_args_x86_uses_tty_s0_and_no_auto_test() {
        let s = boot_args(Arch::X86_64, false);
        assert!(s.starts_with("console=ttyS0 "));
        assert!(!s.contains("auto_test"));
    }
}
