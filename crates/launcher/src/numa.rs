//! NUMA 拓扑：`nodes` × `memory_per_node`，`smp` 必须被 nodes 整除
//! （非法配置在解析期报错，而非运行时）。

/// NUMA 拓扑。
#[derive(Debug, Clone)]
pub struct NumaTopology {
    pub smp: u32,
    pub nodes: u32,
    /// 每节点内存，如 "1G"；总内存 = per-node × nodes
    pub memory_per_node: String,
}

impl NumaTopology {
    pub fn parse(smp: &str, nodes: &str, memory_per_node: &str) -> Result<NumaTopology, String> {
        let smp: u32 = smp
            .trim()
            .parse()
            .map_err(|_| format!("SMP 不是数字: {smp}"))?;
        let nodes: u32 = nodes
            .trim()
            .parse()
            .map_err(|_| format!("NUMA_NODES 不是数字: {nodes}"))?;
        if nodes == 0 {
            return Err("NUMA_NODES 不能为 0".into());
        }
        if nodes > 1 && !smp.is_multiple_of(nodes) {
            return Err(format!("SMP ({smp}) not divisible by NUMA_NODES ({nodes})"));
        }
        if memory_per_node.is_empty() {
            return Err("NUMA_MEMORY 为空".into());
        }
        Ok(NumaTopology { smp, nodes, memory_per_node: memory_per_node.to_string() })
    }

    /// 总内存字符串（run-qemu.sh 的乘法规则：数字前缀 × 节点数，保留单位后缀）。
    /// 单位表与 firecracker 的 MiB 换算共用 common::units。
    pub fn total_memory(&self) -> Result<String, String> {
        let (n, unit) = common::units::parse_memory(&self.memory_per_node)?;
        let total = n.saturating_mul(self.nodes as u64);
        Ok(unit.render(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numa_divisibility_enforced() {
        assert!(NumaTopology::parse("7", "2", "1G").is_err());
        assert!(NumaTopology::parse("8", "2", "1G").is_ok());
        assert!(NumaTopology::parse("x", "2", "1G").is_err());
        assert!(NumaTopology::parse("8", "0", "1G").is_err());
    }

    #[test]
    fn total_memory_suffix_rules() {
        assert_eq!(NumaTopology::parse("8", "2", "1G").unwrap().total_memory().unwrap(), "2G");
        assert_eq!(NumaTopology::parse("8", "2", "512M").unwrap().total_memory().unwrap(), "1024M");
        assert_eq!(NumaTopology::parse("8", "2", "1024").unwrap().total_memory().unwrap(), "2048");
        assert!(NumaTopology::parse("8", "2", "1X").unwrap().total_memory().is_err());
    }
}
