//! QEMU 启动参数 DSL —— run-qemu.sh 的强类型等价物。

use anyhow::Context;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use common::{Arch, HostOs};

use crate::numa::NumaTopology;
pub use crate::{DataDisk, PmemSpec};

/// 加速器：KVM（Linux）/ HVF（macOS）仅宿主与目标同构时可用（调用方校验），
/// 交叉架构回退 TCG。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accel {
    Tcg,
    Kvm,
    Hvf,
}

impl Accel {
    /// verdict.json 运行指纹里的呈现名。
    pub fn label(self) -> &'static str {
        match self {
            Accel::Tcg => "TCG",
            Accel::Kvm => "KVM",
            Accel::Hvf => "HVF",
        }
    }

    /// 平台缺省加速器：macOS 上宿主架构与目标同构 → HVF（Apple Silicon 近
    /// 原生），其余（含 Linux 全部场景）→ TCG。Linux 的 KVM 仍由 `shell
    /// --kvm` 显式开启；macOS 想强制纯模拟用 `--tcg`。宿主架构显式传参
    /// （运行时取 `Arch::host_default()`）而非函数内自查——编译目标推断会让
    /// 钉死他平台行为的单测在 CI 的另一架构上必红。
    pub fn default_for(arch: Arch, host: HostOs, host_arch: Option<Arch>) -> Accel {
        if host == HostOs::Darwin && host_arch == Some(arch) {
            Accel::Hvf
        } else {
            Accel::Tcg
        }
    }
}

/// QEMU 启动参数。
#[derive(Debug, Clone)]
pub struct QemuInvocation {
    pub arch: Arch,
    pub qemu_override: Option<String>,
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub rootfs: PathBuf,
    /// 额外数据盘（virtio-blk，追加于 rootfs 之后 → /dev/vdb 起），
    /// 如 tools.img（挂到 /tools 供 PATH 引用）。
    pub data_disks: Vec<DataDisk>,
    pub accel: Accel,
    /// 宿主平台：决定内存后端形态（Linux=memfd、macOS=ram）与 accel 合法性。
    /// 缺省取编译目标；argv_* 单测显式钉死以冻结双基线。
    pub host: HostOs,
    pub topo: NumaTopology,
    /// kernel cmdline 的 auto_test 开关
    pub auto_test: bool,
    /// `-s -S`：挂起等待 GDB 连接 :1234
    pub gdb_stub: bool,
    /// AI agent 通道：virtio-serial 端口，宿主侧 unix socket（chardev）。
    /// Some 时追加 chardev + virtio-serial-pci + virtserialport；None（缺省）
    /// argv 与 run-qemu.sh 基线逐字不变（冻结不变量 3）。
    pub agent_serial: Option<PathBuf>,
    /// 持久内存组件（DT 途径）：Some 时主内存后端换成宿主文件（share=on，
    /// pmem 区写入持久落盘）并追加 -dtb（补丁后的设备树，含 pmem-region
    /// 节点）；None（缺省）argv 与基线逐字不变（冻结不变量 3）。
    /// 仅单节点 NUMA 支持；x86_64 无 DT 由调用方拒绝。
    pub pmem: Option<PmemSpec>,
    /// 原样透传（shell 展开语义：按空白切分）
    pub extra_opts: Vec<String>,
}

impl QemuInvocation {
    pub fn new(
        arch: Arch,
        kernel: impl Into<PathBuf>,
        initrd: impl Into<PathBuf>,
        rootfs: impl Into<PathBuf>,
    ) -> Self {
        Self {
            arch,
            qemu_override: None,
            kernel: kernel.into(),
            initrd: initrd.into(),
            rootfs: rootfs.into(),
            data_disks: Vec::new(),
            accel: Accel::Tcg,
            host: HostOs::current(),
            topo: NumaTopology {
                smp: 1,
                nodes: 1,
                memory_per_node: "1G".into(),
            },
            auto_test: false,
            gdb_stub: false,
            agent_serial: None,
            pmem: None,
            extra_opts: Vec::new(),
        }
    }

    pub fn accel(mut self, accel: Accel) -> Self {
        self.accel = accel;
        self
    }

    /// 钉死宿主平台（argv 冻结单测用；运行路径缺省取编译目标）。
    pub fn host_os(mut self, host: HostOs) -> Self {
        self.host = host;
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

    /// 附加一个 virtio-blk 数据盘（按调用顺序出现在 /dev/vdb、/dev/vdc…）。
    pub fn virtio_disk(mut self, disk: DataDisk) -> Self {
        self.data_disks.push(disk);
        self
    }

    /// 批量附加数据盘（接受 Option / 迭代器，空缺省不变）。
    pub fn virtio_disks(mut self, disks: impl IntoIterator<Item = DataDisk>) -> Self {
        self.data_disks.extend(disks);
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

    /// 启用 AI agent 通道（virtio-serial；宿主通过该 unix socket 与 guest
    /// 内 virtuoso-agent 通信）。socket 文件须不存在（QEMU 不会清理旧路径）。
    pub fn agent_serial(mut self, sock: impl Into<PathBuf>) -> Self {
        self.agent_serial = Some(sock.into());
        self
    }

    /// 附加持久内存设备（None = 缺省，argv 保持基线）。
    pub fn pmem(mut self, spec: Option<PmemSpec>) -> Self {
        self.pmem = spec;
        self
    }

    pub fn extra_opts(mut self, opts: &[String]) -> Self {
        self.extra_opts = opts.to_vec();
        self
    }

    pub fn qemu_bin(&self) -> String {
        self.qemu_override
            .clone()
            .unwrap_or_else(|| self.arch.qemu_bin().to_string())
    }

    /// 内核 cmdline（run-qemu.sh 冻结文本；pmem 组件追加 mem= 把 pmem 区
    /// 从内核线性内存模型中排除，等价 x86 memmap= 语义）。
    pub fn cmdline(&self) -> String {
        let mut cmd = format!(
            "console={} root=/dev/vda rw init=/init loglevel=8",
            self.arch.console()
        );
        if self.auto_test {
            cmd.push_str(" auto_test");
        }
        if let Some(pmem) = &self.pmem {
            cmd.push_str(&format!(" mem={}", pmem.mem_limit));
        }
        cmd
    }

    /// 完整 argv，与 run-qemu.sh 的展开顺序逐字对齐：
    /// machine, memory-backend, numa, kvm, cpu, smp, m, kernel, initrd,
    /// append, rootfs drive, data disks, extra, console, options, debug。
    pub fn argv(&self) -> anyhow::Result<Vec<String>> {
        // accel × 宿主平台合法性（错误前置到 argv 构造期，而非留给 QEMU 报）
        match (self.accel, self.host) {
            (Accel::Kvm, HostOs::Darwin) => {
                anyhow::bail!("KVM 需要 Linux 宿主（macOS 硬件加速是 HVF）");
            }
            (Accel::Hvf, HostOs::Linux) => {
                anyhow::bail!("HVF 需要 macOS 宿主（Linux 硬件加速是 KVM）");
            }
            _ => {}
        }
        let total_mem = self.topo.total_memory()?;
        // Linux 基线冻结用 memfd 后端；macOS QEMU 无 memfd_create，换 ram
        //（形态等价：同为普通匿名内存，share 语义 ram 恒 off 不需显式声明）
        let mem_backend = |id: &str, size: &str| -> String {
            match self.host {
                HostOs::Linux => {
                    format!("memory-backend-memfd,id={id},size={size},share=off")
                }
                HostOs::Darwin => format!("memory-backend-ram,id={id},size={size}"),
            }
        };
        let mut args: Vec<String> = Vec::new();

        args.push("-machine".into());
        if self.topo.nodes > 1 {
            if self.pmem.is_some() {
                anyhow::bail!("pmem 组件暂不支持 NUMA 多节点（保留单节点拓扑）");
            }
            args.push(self.arch.machine().into());
        } else {
            args.push(format!("{},memory-backend=mem", self.arch.machine()));
        }

        if self.topo.nodes > 1 {
            let per_node = self.topo.smp / self.topo.nodes;
            for i in 0..self.topo.nodes {
                args.push("-object".into());
                args.push(mem_backend(&format!("mem{i}"), &self.topo.memory_per_node));
                let start = i * per_node;
                let end = start + per_node - 1;
                args.push("-numa".into());
                args.push(format!("node,nodeid={i},memdev=mem{i},cpus={start}-{end}"));
            }
        } else if let Some(pmem) = &self.pmem {
            // 主内存换文件后端（share=on）：DT 挖出的 pmem 区即宿主文件
            // backed，guest 写入持久落盘（两平台同形）
            args.push("-object".into());
            args.push(format!(
                "memory-backend-file,id=mem,mem-path={},size={total_mem},share=on",
                pmem.ram_backend.display()
            ));
        } else {
            args.push("-object".into());
            args.push(mem_backend("mem", &total_mem));
        }

        match self.accel {
            Accel::Kvm => args.push("-enable-kvm".into()),
            Accel::Hvf => {
                args.push("-accel".into());
                args.push("hvf".into());
            }
            Accel::Tcg => {}
        }
        args.push("-cpu".into());
        args.push(match self.accel {
            Accel::Kvm | Accel::Hvf => "host".into(),
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

        if let Some(pmem) = &self.pmem {
            // 补丁后的设备树（/memory 挖出 pmem 区 + 根节点 pmem-region），
            // 替换 QEMU 生成版 —— of_pmem 据此注册 /dev/pmem0
            args.push("-dtb".into());
            args.push(pmem.dtb.display().to_string());
        }

        args.push("-drive".into());
        args.push(format!(
            "file={},format=raw,if=virtio",
            self.rootfs.display()
        ));

        for disk in &self.data_disks {
            args.push("-drive".into());
            args.push(format!("file={},format=raw,if=virtio", disk.path.display()));
        }

        if let Some(sock) = &self.agent_serial {
            args.push("-chardev".into());
            args.push(format!(
                "socket,id=agentch,path={},server=on,wait=off",
                sock.display()
            ));
            args.push("-device".into());
            args.push("virtio-serial-pci,id=virtio-serial0".into());
            args.push("-device".into());
            args.push("virtserialport,chardev=agentch,id=agentport,name=virtuoso-agent".into());
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
    pub fn command_line(&self) -> anyhow::Result<String> {
        let mut parts = vec![self.qemu_bin()];
        parts.extend(self.argv()?.iter().map(|a| common::shell::quote(a)));
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
        ]
        .into_iter()
        .chain(self.data_disks.iter().map(|d| ("Data disk image", &d.path)))
        {
            if !path.is_file() {
                anyhow::bail!(
                    "{what} not found at {} (build the kernel / run `virtuoso build` first)",
                    path.display()
                );
            }
        }
        let args = self.argv()?;
        let mut cmd = Command::new(self.qemu_bin());
        cmd.args(&args).process_group(0);
        if piped {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null());
        }
        let child = cmd.spawn().with_context(|| {
            format!(
                "{} not found; run `virtuoso doctor --verbose` for install hints",
                self.qemu_bin()
            )
        })?;
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
    use common::HostOs;

    fn base_inv() -> QemuInvocation {
        QemuInvocation::new(
            Arch::Arm64,
            "/tmp/vmlinuz",
            "/tmp/initrd.img",
            "/tmp/rootfs.img",
        )
        .topo(NumaTopology::parse("8", "2", "1G").unwrap())
        // 冻结基线按宿主平台各持一份：单测显式钉死，不随编译目标漂移
        .host_os(HostOs::Linux)
    }

    #[test]
    fn argv_frozen_baseline_multi_node() {
        let args = base_inv().auto_test(true).argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-machine virt "));
        assert!(joined.contains("-object memory-backend-memfd,id=mem0,size=1G,share=off"));
        assert!(joined.contains("-numa node,nodeid=0,memdev=mem0,cpus=0-3"));
        assert!(joined.contains("-numa node,nodeid=1,memdev=mem1,cpus=4-7"));
        assert!(joined.contains("-smp 8,sockets=2,cores=4,threads=1"));
        assert!(joined.contains("-cpu cortex-a72"));
        assert!(joined.contains("-m 2G"));
        assert!(joined
            .contains("-append console=ttyAMA0 root=/dev/vda rw init=/init loglevel=8 auto_test"));
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
            .topo(NumaTopology::parse("4", "1", "2G").unwrap())
            .host_os(HostOs::Linux);
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
    fn argv_darwin_frozen_baseline_uses_ram_backend() {
        // darwin 基线：无 memfd（macOS QEMU 未编译该后端），单节点与
        // NUMA 每节点均换 memory-backend-ram（ram 恒非共享，无 share= 字段）
        let args = base_inv()
            .host_os(HostOs::Darwin)
            .topo(NumaTopology::parse("8", "1", "2G").unwrap())
            .auto_test(true)
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-machine virt,memory-backend=mem"));
        assert!(joined.contains("-object memory-backend-ram,id=mem,size=2G"));
        assert!(!joined.contains("memfd"));
        assert!(!joined.contains("share=off"));
        assert!(joined.contains("-cpu cortex-a72"));
    }

    #[test]
    fn argv_darwin_numa_uses_ram_backend_per_node() {
        let args = base_inv().host_os(HostOs::Darwin).argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-object memory-backend-ram,id=mem0,size=1G"));
        assert!(joined.contains("-object memory-backend-ram,id=mem1,size=1G"));
        assert!(!joined.contains("memfd"));
    }

    #[test]
    fn hvf_uses_accel_flag_and_host_cpu() {
        let args = base_inv()
            .host_os(HostOs::Darwin)
            .accel(Accel::Hvf)
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-accel hvf -cpu host"));
        assert!(!joined.contains("cortex-a72"));
        assert!(!joined.contains("-enable-kvm"));
    }

    #[test]
    fn kvm_rejected_on_darwin_and_hvf_rejected_on_linux() {
        let kvm_on_mac = base_inv()
            .host_os(HostOs::Darwin)
            .accel(Accel::Kvm)
            .argv()
            .unwrap_err();
        assert!(kvm_on_mac.to_string().contains("HVF"));
        let hvf_on_linux = base_inv().accel(Accel::Hvf).argv().unwrap_err();
        assert!(hvf_on_linux.to_string().contains("KVM"));
    }

    #[test]
    fn default_accel_only_hvf_on_darwin_same_arch() {
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Darwin, Some(Arch::Arm64)),
            Accel::Hvf,
            "Apple Silicon 宿主跑 arm64 guest 缺省 HVF"
        );
        assert_eq!(
            Accel::default_for(Arch::X86_64, HostOs::Darwin, Some(Arch::Arm64)),
            Accel::Tcg,
            "Apple Silicon 宿主交叉 x86_64 guest 回落 TCG"
        );
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Darwin, None),
            Accel::Tcg,
            "宿主架构未知回落 TCG"
        );
        assert_eq!(
            Accel::default_for(Arch::Arm64, HostOs::Linux, Some(Arch::Arm64)),
            Accel::Tcg,
            "Linux 全场景缺省 TCG（KVM 仍由 --kvm 显式开启）"
        );
    }

    #[test]
    fn virtio_disk_and_gdb_and_extra_opts() {
        let args = base_inv()
            .virtio_disk(DataDisk::new("/tmp/tools.img"))
            .gdb_stub(true)
            .extra_opts(&["-device vfio-pci,host=00:01.0".to_string()])
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-drive file=/tmp/tools.img,format=raw,if=virtio"));
        // NVMe 测试盘已移除：不再出现 blockdev/nvme 设备
        assert!(!joined.contains("-blockdev"));
        assert!(!joined.contains("nvme"));
        assert!(joined.contains("-s -S"));
        assert!(joined.contains("-device vfio-pci,host=00:01.0"));
        // 顺序：数据盘紧跟 rootfs drive 之后、extra_opts 之前
        let pos = |p: &str| args.iter().position(|a| a.contains(p)).unwrap();
        assert!(pos("file=/tmp/rootfs.img") < pos("file=/tmp/tools.img"));
    }

    #[test]
    fn multiple_data_disks_append_in_order() {
        let args = base_inv()
            .virtio_disk(DataDisk::new("/tmp/tools.img"))
            .virtio_disk(DataDisk::new("/tmp/data0.img"))
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined
            .contains("-drive file=/tmp/rootfs.img,format=raw,if=virtio -drive file=/tmp/tools.img,format=raw,if=virtio -drive file=/tmp/data0.img,format=raw,if=virtio"));
    }

    #[test]
    fn data_disks_coexist_with_agent_serial() {
        let args = base_inv()
            .virtio_disk(DataDisk::new("/tmp/tools.img"))
            .agent_serial("/tmp/virtuoso-agent.sock")
            .argv()
            .unwrap();
        let pos = |p: &str| args.iter().position(|a| a.contains(p)).unwrap();
        assert!(pos("-drive") < pos("-chardev"));
    }

    #[test]
    fn no_data_disks_keeps_baseline_argv() {
        // 冻结不变量 3：缺省（无数据盘）argv 与 run-qemu.sh 基线逐字一致
        let args = base_inv().argv().unwrap();
        let joined = args.join(" ");
        assert_eq!(
            joined.matches("-drive").count(),
            1,
            "只有 rootfs 一个 -drive"
        );
        assert!(!joined.contains("-blockdev"));
        assert!(!joined.contains("nvdimm"), "缺省不得出现 nvdimm");
    }

    #[test]
    fn pmem_swaps_ram_backend_adds_dtb_and_mem_limit() {
        // 单节点：主内存换文件后端（share=on）+ -dtb + cmdline mem=
        let inv = QemuInvocation::new(Arch::X86_64, "k", "i", "r")
            .topo(NumaTopology::parse("4", "1", "2G").unwrap())
            .host_os(HostOs::Linux)
            .pmem(Some(PmemSpec::new(
                "256M",
                "1792M",
                "/tmp/ram.img",
                "/tmp/virt.dtb",
            )));
        let args = inv.argv().unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-machine virt,memory-backend=mem"));
        assert!(!joined.contains("nvdimm"), "DT 途径不使用 QEMU nvdimm 设备");
        assert!(joined
            .contains("-object memory-backend-file,id=mem,mem-path=/tmp/ram.img,size=2G,share=on"));
        assert!(joined.contains("-dtb /tmp/virt.dtb"));
        assert!(joined.contains("mem=1792M"));
        // 顺序：-dtb 跟在 -append 之后、-drive 之前
        let pos = |p: &str| args.iter().position(|a| a.contains(p)).unwrap();
        assert!(pos("-append") < pos("-dtb"));
        assert!(pos("-dtb") < pos("-drive"));
    }

    #[test]
    fn pmem_rejected_with_multi_node_numa() {
        let err = base_inv()
            .pmem(Some(PmemSpec::new(
                "256M",
                "1792M",
                "/tmp/ram.img",
                "/tmp/virt.dtb",
            )))
            .argv()
            .unwrap_err();
        assert!(err.to_string().contains("NUMA"));
    }

    #[test]
    fn cmdline_without_auto_test_has_no_trailing_space() {
        let inv = QemuInvocation::new(Arch::Riscv64, "k", "i", "r");
        assert_eq!(
            inv.cmdline(),
            "console=ttyS0 root=/dev/vda rw init=/init loglevel=8"
        );
    }

    #[test]
    fn agent_serial_off_by_default_keeps_baseline_argv() {
        // 冻结不变量 3：未启用 agent 通道时 argv 与基线逐字一致
        let joined = base_inv().argv().unwrap().join(" ");
        assert!(!joined.contains("virtio-serial"));
        assert!(!joined.contains("chardev"));
    }

    #[test]
    fn agent_serial_adds_chardev_and_virtserialport() {
        let args = base_inv()
            .agent_serial("/tmp/virtuoso-agent.sock")
            .argv()
            .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains(
            "-chardev socket,id=agentch,path=/tmp/virtuoso-agent.sock,server=on,wait=off"
        ));
        assert!(joined.contains("-device virtio-serial-pci,id=virtio-serial0"));
        assert!(joined
            .contains("-device virtserialport,chardev=agentch,id=agentport,name=virtuoso-agent"));
        // 顺序约束：agent 设备在盘之后、-nographic 之前
        let pos = |p: &str| args.iter().position(|a| a.contains(p)).unwrap();
        assert!(pos("-drive") < pos("-chardev"));
        assert!(pos("virtserialport") < pos("-nographic"));
    }

    #[test]
    fn command_line_quotes_append_value() {
        let args = base_inv().command_line().unwrap();
        // 路径与逗号安全字符原样；含空格的 -append 值整体单引号
        assert!(args.contains("-drive file=/tmp/rootfs.img,format=raw,if=virtio"));
        assert!(args.contains("-append 'console=ttyAMA0 root=/dev/vda rw init=/init loglevel=8'"));
        assert!(args.starts_with("qemu-system-aarch64 -machine virt"));
    }
}
