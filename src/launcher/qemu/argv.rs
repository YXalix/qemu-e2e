//! argv 装配：完整 argv 与 run-qemu.sh 的展开顺序逐字对齐
//! （machine, memory-backend, numa, kvm, cpu, smp, m, kernel, initrd,
//! append, rootfs drive, data disks, extra, console, options, debug）。

use crate::HostOs;

use super::{Accel, QemuInvocation};

impl QemuInvocation {
    /// 完整 argv，与 run-qemu.sh 的展开顺序逐字对齐：
    /// machine, memory-backend, numa, kvm, cpu, smp, m, kernel, initrd,
    /// append, rootfs drive, data disks, extra, console, options, debug。
    pub(crate) fn argv(&self) -> anyhow::Result<Vec<String>> {
        // accel × 宿主平台合法性（错误前置到 argv 构造期，而非留给 QEMU 报）
        match (self.accel, self.host) {
            (Accel::Kvm, HostOs::Darwin) => {
                anyhow::bail!(
                    "KVM requires a Linux host (on macOS the hardware accelerator is HVF)"
                );
            }
            (Accel::Hvf, HostOs::Linux) => {
                anyhow::bail!(
                    "HVF requires a macOS host (on Linux the hardware accelerator is KVM)"
                );
            }
            _ => {}
        }
        let total_mem = self.topo.total_memory()?;
        let mut args: Vec<String> = Vec::new();

        self.push_machine_and_memory(&mut args, &total_mem)?;
        self.push_accel_cpu_smp(&mut args);
        self.push_boot_and_drives(&mut args, &total_mem);
        self.push_agent_channel(&mut args);
        self.push_tail(&mut args);
        Ok(args)
    }

    /// -machine + 内存后端（单节点 memory-backend=mem 关联 / 多节点逐节点
    /// -object+-numa / pmem 换文件后端 share=on）。Linux 基线冻结用 memfd
    /// 后端；macOS QEMU 无 memfd_create，换 ram（形态等价：同为普通匿名
    /// 内存，share 语义 ram 恒 off 不需显式声明）。
    fn mem_backend(&self, id: &str, size: &str) -> String {
        match self.host {
            HostOs::Linux => format!("memory-backend-memfd,id={id},size={size},share=off"),
            HostOs::Darwin => format!("memory-backend-ram,id={id},size={size}"),
        }
    }

    fn push_machine_and_memory(
        &self,
        args: &mut Vec<String>,
        total_mem: &str,
    ) -> anyhow::Result<()> {
        args.push("-machine".into());
        if self.topo.nodes > 1 {
            if self.pmem.is_some() {
                anyhow::bail!("the pmem component does not support multi-node NUMA yet (single-node topology kept)");
            }
            args.push(self.arch.machine().into());
        } else {
            args.push(format!("{},memory-backend=mem", self.arch.machine()));
        }

        if self.topo.nodes > 1 {
            let per_node = self.topo.smp / self.topo.nodes;
            for i in 0..self.topo.nodes {
                args.push("-object".into());
                args.push(self.mem_backend(&format!("mem{i}"), &self.topo.memory_per_node));
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
            args.push(self.mem_backend("mem", total_mem));
        }
        Ok(())
    }

    /// 加速器开关 + -cpu + -smp。
    fn push_accel_cpu_smp(&self, args: &mut Vec<String>) {
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
    }

    /// -m / -kernel / -initrd / -append /（pmem -dtb）/ rootfs drive / 数据盘。
    fn push_boot_and_drives(&self, args: &mut Vec<String>, total_mem: &str) {
        args.push("-m".into());
        args.push(total_mem.to_string());
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
    }

    /// AI agent 通道（virtio-serial chardev + 端口设备）。
    fn push_agent_channel(&self, args: &mut Vec<String>) {
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
    }

    /// extra_opts 透传与收尾参数（console / -no-reboot / gdb stub）。
    fn push_tail(&self, args: &mut Vec<String>) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::{DataDisk, PmemSpec};
    use crate::launcher::numa::NumaTopology;
    use crate::{Arch, HostOs};

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
}
