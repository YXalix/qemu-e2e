//! QEMU 启动参数 DSL —— run-qemu.sh 的强类型等价物。

use anyhow::Context;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use common::Arch;

use crate::numa::NumaTopology;

/// 加速器：KVM 仅宿主与目标同构时可用（调用方校验），交叉架构回退 TCG。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accel {
    Tcg,
    Kvm,
}

/// QEMU 启动参数。
#[derive(Debug, Clone)]
pub struct QemuInvocation {
    pub arch: Arch,
    pub qemu_override: Option<String>,
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub rootfs: PathBuf,
    /// 可选 NVMe 测试盘（disk.qcow2 → /dev/nvme0n1）
    pub disk: Option<PathBuf>,
    pub accel: Accel,
    pub topo: NumaTopology,
    /// kernel cmdline 的 auto_test 开关
    pub auto_test: bool,
    /// `-s -S`：挂起等待 GDB 连接 :1234
    pub gdb_stub: bool,
    /// 原样透传（shell 展开语义：按空白切分）
    pub extra_opts: Vec<String>,
}

impl QemuInvocation {
    pub fn new(arch: Arch, kernel: impl Into<PathBuf>, initrd: impl Into<PathBuf>, rootfs: impl Into<PathBuf>) -> Self {
        Self {
            arch,
            qemu_override: None,
            kernel: kernel.into(),
            initrd: initrd.into(),
            rootfs: rootfs.into(),
            disk: None,
            accel: Accel::Tcg,
            topo: NumaTopology { smp: 1, nodes: 1, memory_per_node: "1G".into() },
            auto_test: false,
            gdb_stub: false,
            extra_opts: Vec::new(),
        }
    }

    pub fn accel(mut self, accel: Accel) -> Self {
        self.accel = accel;
        self
    }

    pub fn topo(mut self, topo: NumaTopology) -> Self {
        self.topo = topo;
        self
    }

    pub fn qemu_override(mut self, qemu: Option<&str>) -> Self {
        self.qemu_override = qemu.filter(|s| !s.is_empty()).map(str::to_string);
        self
    }

    pub fn disk(mut self, disk: Option<impl Into<PathBuf>>) -> Self {
        self.disk = disk.map(Into::into);
        self
    }

    pub fn auto_test(mut self, on: bool) -> Self {
        self.auto_test = on;
        self
    }

    pub fn gdb_stub(mut self, on: bool) -> Self {
        self.gdb_stub = on;
        self
    }

    pub fn extra_opts(mut self, opts: &[String]) -> Self {
        self.extra_opts = opts.to_vec();
        self
    }

    pub fn qemu_bin(&self) -> String {
        self.qemu_override.clone().unwrap_or_else(|| self.arch.qemu_bin().to_string())
    }

    /// 内核 cmdline（run-qemu.sh 冻结文本）。
    pub fn cmdline(&self) -> String {
        let mut cmd = format!(
            "console={} root=/dev/vda rw init=/init loglevel=8",
            self.arch.console()
        );
        if self.auto_test {
            cmd.push_str(" auto_test");
        }
        cmd
    }

    /// 完整 argv，与 run-qemu.sh 的展开顺序逐字对齐：
    /// machine, memory-backend, numa, kvm, cpu, smp, m, kernel, initrd,
    /// append, rootfs drive, nvme disk, extra, console, options, debug。
    pub fn argv(&self) -> Result<Vec<String>, String> {
        let total_mem = self.topo.total_memory()?;
        let mut args: Vec<String> = Vec::new();

        args.push("-machine".into());
        if self.topo.nodes > 1 {
            args.push(self.arch.machine().into());
        } else {
            args.push(format!("{},memory-backend=mem", self.arch.machine()));
        }

        if self.topo.nodes > 1 {
            let per_node = self.topo.smp / self.topo.nodes;
            for i in 0..self.topo.nodes {
                args.push("-object".into());
                args.push(format!(
                    "memory-backend-memfd,id=mem{i},size={},share=off",
                    self.topo.memory_per_node
                ));
                let start = i * per_node;
                let end = start + per_node - 1;
                args.push("-numa".into());
                args.push(format!("node,nodeid={i},memdev=mem{i},cpus={start}-{end}"));
            }
        } else {
            args.push("-object".into());
            args.push(format!(
                "memory-backend-memfd,id=mem,size={total_mem},share=off"
            ));
        }

        if self.accel == Accel::Kvm {
            args.push("-enable-kvm".into());
        }
        args.push("-cpu".into());
        args.push(match self.accel {
            Accel::Kvm => "host".into(),
            Accel::Tcg => self.arch.cpu_tcg().into(),
        });

        args.push("-smp".into());
        if self.topo.nodes > 1 {
            let per_node = self.topo.smp / self.topo.nodes;
            args.push(format!(
                "{},sockets={},cores={},threads=1",
                self.topo.smp, self.topo.nodes, per_node
            ));
        } else {
            args.push(self.topo.smp.to_string());
        }

        args.push("-m".into());
        args.push(total_mem);
        args.push("-kernel".into());
        args.push(self.kernel.display().to_string());
        args.push("-initrd".into());
        args.push(self.initrd.display().to_string());
        args.push("-append".into());
        args.push(self.cmdline());

        args.push("-drive".into());
        args.push(format!("file={},format=raw,if=virtio", self.rootfs.display()));

        if let Some(disk) = &self.disk {
            args.push("-blockdev".into());
            args.push(format!(
                "driver=qcow2,file.driver=file,file.filename={},node-name=ssd0,discard=unmap,file.discard=unmap,file.locking=off",
                disk.display()
            ));
            args.push("-device".into());
            args.push("nvme,drive=ssd0,serial=nvme-ssd-0".into());
        }

        for opt in &self.extra_opts {
            args.extend(opt.split_whitespace().map(str::to_string));
        }

        args.push("-nographic".into());
        args.push("-serial".into());
        args.push("mon:stdio".into());
        args.push("-no-reboot".into());

        if self.gdb_stub {
            args.push("-s".into());
            args.push("-S".into());
        }
        Ok(args)
    }

    /// 单行可复制启动命令（shell 引用；spawn 前展示 / 手动复现用）。
    pub fn command_line(&self) -> Result<String, String> {
        let mut parts = vec![self.qemu_bin()];
        parts.extend(self.argv()?.iter().map(|a| crate::shell_quote(a)));
        Ok(parts.join(" "))
    }

    /// spawn QEMU：独立进程组（pgid = 返回的 child pid，交给 guardian 收割）。
    /// `piped` = true 时 stdout/stderr 管道化（test 路径捕获串口），
    /// false 时继承宿主 stdio（交互 shell / debug）。
    pub fn spawn(self, piped: bool) -> anyhow::Result<(Child, u32)> {
        for (what, path) in [
            ("Kernel image", &self.kernel),
            ("Initramfs", &self.initrd),
            ("Rootfs image", &self.rootfs),
        ] {
            if !path.is_file() {
                anyhow::bail!("{what} not found at {} (build the kernel / run `cargo xtask build` first)", path.display());
            }
        }
        let args = self.argv().map_err(anyhow::Error::msg)?;
        let mut cmd = Command::new(self.qemu_bin());
        cmd.args(&args).process_group(0);
        if piped {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("{} not found; run `cargo xtask verify` for install hints", self.qemu_bin()))?;
        let pgid = child.id();
        Ok((child, pgid))
    }

    /// spawn 并登记监管：注册表 + 收割守卫一步到位（取代调用方四步样板）。
    pub fn spawn_supervised(self, piped: bool) -> anyhow::Result<(Child, guardian::Supervised)> {
        let (child, pgid) = self.spawn(piped)?;
        Ok((child, guardian::Supervised::adopt(pgid)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numa::NumaTopology;

    fn base_inv() -> QemuInvocation {
        QemuInvocation::new(Arch::Arm64, "/tmp/vmlinuz", "/tmp/initrd.img", "/tmp/rootfs.img")
            .topo(NumaTopology::parse("8", "2", "1G").unwrap())
    }

    #[test]
    fn argv_matches_run_qemu_sh_multi_node() {
        let args = base_inv().auto_test(true).argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-machine virt "));
        assert!(joined.contains("-object memory-backend-memfd,id=mem0,size=1G,share=off"));
        assert!(joined.contains("-numa node,nodeid=0,memdev=mem0,cpus=0-3"));
        assert!(joined.contains("-numa node,nodeid=1,memdev=mem1,cpus=4-7"));
        assert!(joined.contains("-smp 8,sockets=2,cores=4,threads=1"));
        assert!(joined.contains("-cpu cortex-a72"));
        assert!(joined.contains("-m 2G"));
        assert!(joined.contains(
            "-append console=ttyAMA0 root=/dev/vda rw init=/init loglevel=8 auto_test"
        ));
        assert!(joined.contains("-drive file=/tmp/rootfs.img,format=raw,if=virtio"));
        assert!(joined.contains("-nographic -serial mon:stdio -no-reboot"));
        // 顺序约束：machine 必须最先，-no-reboot 在 console 之后
        assert_eq!(args[0], "-machine");
        let pos = |p: &str| args.iter().position(|a| a == p).unwrap();
        assert!(pos("-serial") < pos("-no-reboot"));
    }

    #[test]
    fn argv_single_node_uses_memory_backend_on_machine() {
        let inv = QemuInvocation::new(Arch::X86_64, "k", "i", "r")
            .topo(NumaTopology::parse("4", "1", "2G").unwrap());
        let args = inv.argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-machine virt,memory-backend=mem"));
        assert!(joined.contains("memory-backend-memfd,id=mem,size=2G,share=off"));
        assert!(joined.contains("-smp 4"));
        assert!(joined.contains("-cpu qemu64"));
        assert!(joined.contains("console=ttyS0"));
        assert!(!joined.contains("-numa"));
    }

    #[test]
    fn kvm_uses_host_cpu_and_enable_flag() {
        let args = base_inv().accel(Accel::Kvm).argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-enable-kvm -cpu host"));
        assert!(!joined.contains("cortex-a72"));
    }

    #[test]
    fn disk_and_gdb_and_extra_opts() {
        let args = base_inv()
            .disk(Some("/tmp/disk.qcow2"))
            .gdb_stub(true)
            .extra_opts(&["-device vfio-pci,host=00:01.0".to_string()])
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains(
            "-blockdev driver=qcow2,file.driver=file,file.filename=/tmp/disk.qcow2,node-name=ssd0,discard=unmap,file.discard=unmap,file.locking=off -device nvme,drive=ssd0,serial=nvme-ssd-0"
        ));
        assert!(joined.contains("-s -S"));
        assert!(joined.contains("-device vfio-pci,host=00:01.0"));
    }

    #[test]
    fn cmdline_without_auto_test_has_no_trailing_space() {
        let inv = QemuInvocation::new(Arch::Riscv64, "k", "i", "r");
        assert_eq!(inv.cmdline(), "console=ttyS0 root=/dev/vda rw init=/init loglevel=8");
    }

    #[test]
    fn command_line_quotes_append_value() {
        let args = base_inv().command_line().unwrap();
        // 路径与逗号安全字符原样；含空格的 -append 值整体单引号
        assert!(args.contains("-drive file=/tmp/rootfs.img,format=raw,if=virtio"));
        assert!(args.contains(
            "-append 'console=ttyAMA0 root=/dev/vda rw init=/init loglevel=8'"
        ));
        assert!(args.starts_with("qemu-system-aarch64 -machine virt"));
    }
}
