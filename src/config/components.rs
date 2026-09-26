//! 组件访问器：tools_disk / agent / vfio / numa / pmem 的开关判定与
//! 参数投影。组件语义（段缺省行为、env 覆盖）在各访问器 doc 上冻结。

use super::schema::push_enabled;
use super::{scalar, Config};

impl Config {
    /// vfio 直通设备 BDF 列表（组件未启用 = 空）。
    pub fn vfio(&self) -> Option<&[String]> {
        let v = self.comp(|c| c.vfio.as_ref())?;
        v.enabled
            .unwrap_or(false)
            .then(|| v.devices.as_deref().unwrap_or(&[]))
    }

    /// vfio 组件：每设备一条 `-device vfio-pci,host=<bdf>`。
    /// 组件增量由装配点（cli::build_invocation）显式追加，不参与
    /// qemu_opts 的 env/toml 优先级链。
    pub fn vfio_opts(&self) -> Vec<String> {
        match self.vfio() {
            Some(devices) if !devices.is_empty() => devices
                .iter()
                .map(|d| format!("-device vfio-pci,host={d}"))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// NUMA 拓扑参数 (smp, nodes, memory_per_node)，供 NumaTopology::parse。
    /// 组件化取值：smp 全局；nodes/memory 来自启用的 [components.numa]，
    /// 未启用（缺省单节点）回落遗留变量，再回落 1 / "1G"。
    pub fn topo_params(&self) -> (String, String, String) {
        let smp = scalar(self.tv(|t| &t.smp), "SMP").unwrap_or_else(|| "8".into());
        let numa = self
            .comp(|c| c.numa.as_ref())
            .filter(|n| n.enabled.unwrap_or(false));
        // 优先级冻结：NUMA_NODES / NUMA_MEMORY env 覆盖 toml（组件启用与否皆然）
        let nodes = match &numa {
            Some(n) => scalar(n.nodes.as_ref(), "NUMA_NODES").unwrap_or_else(|| "2".into()),
            None => std::env::var("NUMA_NODES")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "1".into()),
        };
        let mem = match &numa {
            Some(n) => {
                scalar(n.memory_per_node.as_ref(), "NUMA_MEMORY").unwrap_or_else(|| "1G".into())
            }
            None => std::env::var("NUMA_MEMORY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "1G".into()),
        };
        (smp, nodes, mem)
    }

    /// 组件计划：启用组件的 require 并集（按 schema 固定顺序
    /// tools_disk → agent → vfio → numa → pmem，去重保首个），按 stage 分区。
    /// `[tests]` 段的 require 最后并入（恒 runtime；同名模块组件条目优先，
    /// 测例只补差集）。
    pub fn component_plan(&self) -> super::ComponentPlan {
        let mut plan = super::ComponentPlan::default();
        if let Some(c) = self.comp(|c| c.tools_disk.as_ref()) {
            push_enabled(&mut plan, c);
        }
        if let Some(c) = self.comp(|c| c.agent.as_ref()) {
            push_enabled(&mut plan, c);
        }
        if let Some(c) = self.comp(|c| c.vfio.as_ref()) {
            push_enabled(&mut plan, c);
        }
        if let Some(c) = self.comp(|c| c.numa.as_ref()) {
            push_enabled(&mut plan, c);
        }
        if let Some(c) = self.comp(|c| c.pmem.as_ref()) {
            push_enabled(&mut plan, c);
        }
        if let Some(t) = self.toml.as_ref().and_then(|t| t.tests.as_ref()) {
            plan.push(None, &t.require);
        }
        plan
    }

    /// tools_disk 组件是否启用（段缺省 = true：tools.img 存在即附加，
    /// 与冻结不变量 3 的「调用方增量」语义一致）。
    pub fn tools_disk_enabled(&self) -> bool {
        self.comp(|c| c.tools_disk.as_ref())
            .and_then(|c| c.enabled)
            .unwrap_or(true)
    }

    /// agent 组件是否启用（段缺省 = false：argv 保持冻结基线）。
    pub fn agent_enabled(&self) -> bool {
        self.comp(|c| c.agent.as_ref())
            .and_then(|c| c.enabled)
            .unwrap_or(false)
    }

    /// pmem 组件 size（启用才 Some；未写 size 用缺省 "256M"）。
    /// 段缺省（或 enabled=false）= 关闭：argv 保持冻结基线。
    pub fn pmem_size(&self) -> Option<String> {
        self.comp(|c| c.pmem.as_ref())
            .filter(|p| p.enabled.unwrap_or(false))
            .map(|p| {
                p.size
                    .as_ref()
                    .map(|s| s.0.clone())
                    .unwrap_or_else(|| "256M".into())
            })
    }
}
