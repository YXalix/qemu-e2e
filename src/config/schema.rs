//! toml schema：`virtuoso.toml` 的类型化结构与解析（`mod.rs` 的 `Config`
//! 只做访问器，结构真相在这里）。未知键（`deny_unknown_fields`）与非法类型
//! 一律解析期报错；`StrVal` 统一收编 string/int 标量，`Stage` 区分 KO 的
//! boot/runtime 加载阶段。

use std::path::Path;

use anyhow::Context;

use crate::config::ComponentPlan;

// ---------------------------------------------------------------- toml schema

/// 接受 string 或 integer 标量并统一成 String（timeout_secs = 60 与 = "60"
/// 等价）。其余类型（bool/array…）在解析期报错。
#[derive(Debug, Clone)]
pub(crate) struct StrVal(pub(crate) String);

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

/// 组件 KO 的加载阶段：`boot` = 并入 initramfs（挂 root 前就要），
/// `runtime`（缺省）= switch_root 后由 rootfs init 加载。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Boot,
    Runtime,
}

impl<'de> serde::Deserialize<'de> for Stage {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match String::deserialize(d)?.as_str() {
            "boot" => Ok(Stage::Boot),
            "runtime" => Ok(Stage::Runtime),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["boot", "runtime"],
            )),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtuosoToml {
    pub(crate) kernel_path: Option<StrVal>,
    pub(crate) kernel_image: Option<StrVal>,
    /// kernel_preset = "mainline"：内核走预编供给（virtuoso fetch），免内核树
    pub(crate) kernel_preset: Option<StrVal>,
    pub(crate) arch: Option<StrVal>,
    pub(crate) timeout_secs: Option<StrVal>,
    pub(crate) smp: Option<StrVal>,
    pub(crate) auto_test: Option<bool>,
    pub(crate) qemu: Option<StrVal>,
    pub(crate) qemu_opts: Option<Vec<String>>,
    pub(crate) components: Option<ComponentsSection>,
    pub(crate) busybox: Option<BusyboxSection>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComponentsSection {
    pub(crate) tools_disk: Option<ToolsDiskComponent>,
    pub(crate) agent: Option<AgentComponent>,
    pub(crate) vfio: Option<VfioComponent>,
    pub(crate) numa: Option<NumaComponent>,
    pub(crate) pmem: Option<PmemComponent>,
}

// 组件公共字段（enabled / require / stage）逐结构体显式声明 —— 不用
// #[serde(flatten)]，它与 deny_unknown_fields 不兼容（未知键会漏过）。

/// 组件公共字段的统一视图：启用判定与 require 并集只看这三个面，
/// 各组件结构体逐项实现（五行样板换掉调用方的五行复制块）。
pub(crate) trait Component {
    fn enabled(&self) -> Option<bool>;
    fn stage(&self) -> Option<Stage>;
    fn require(&self) -> &Option<Vec<String>>;
}

/// 启用组件的 require 并入计划（禁用组件整体跳过）。
pub(crate) fn push_enabled(plan: &mut ComponentPlan, c: &impl Component) {
    if c.enabled().unwrap_or(false) {
        plan.push(c.stage(), c.require());
    }
}

/// tools.img 常驻工具盘（外挂 virtio-blk → /dev/vdb 挂 /tools）。
/// 段缺省即启用；require 默认为空（virtio_blk/ext4 已在 boot 基础集）。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsDiskComponent {
    pub(crate) enabled: Option<bool>,
    pub(crate) require: Option<Vec<String>>,
    pub(crate) stage: Option<Stage>,
}

impl Component for ToolsDiskComponent {
    fn enabled(&self) -> Option<bool> {
        self.enabled
    }
    fn stage(&self) -> Option<Stage> {
        self.stage
    }
    fn require(&self) -> &Option<Vec<String>> {
        &self.require
    }
}

/// AI probe 通道（virtio-serial + guest 内 virtuoso-agent）。
/// 段缺省即关闭 —— argv 保持冻结基线（不变量 3）。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentComponent {
    pub(crate) enabled: Option<bool>,
    pub(crate) require: Option<Vec<String>>,
    pub(crate) stage: Option<Stage>,
}

impl Component for AgentComponent {
    fn enabled(&self) -> Option<bool> {
        self.enabled
    }
    fn stage(&self) -> Option<Stage> {
        self.stage
    }
    fn require(&self) -> &Option<Vec<String>> {
        &self.require
    }
}

/// vfio-pci 直通：devices = 宿主设备 BDF 列表，逐条生成
/// `-device vfio-pci,host=<bdf>`。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VfioComponent {
    pub(crate) enabled: Option<bool>,
    pub(crate) require: Option<Vec<String>>,
    pub(crate) stage: Option<Stage>,
    pub(crate) devices: Option<Vec<String>>,
}

impl Component for VfioComponent {
    fn enabled(&self) -> Option<bool> {
        self.enabled
    }
    fn stage(&self) -> Option<Stage> {
        self.stage
    }
    fn require(&self) -> &Option<Vec<String>> {
        &self.require
    }
}

/// 多节点 NUMA 拓扑；段缺省（或 enabled=false）= 单节点。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NumaComponent {
    pub(crate) enabled: Option<bool>,
    pub(crate) require: Option<Vec<String>>,
    pub(crate) stage: Option<Stage>,
    pub(crate) nodes: Option<StrVal>,
    pub(crate) memory_per_node: Option<StrVal>,
}

impl Component for NumaComponent {
    fn enabled(&self) -> Option<bool> {
        self.enabled
    }
    fn stage(&self) -> Option<Stage> {
        self.stage
    }
    fn require(&self) -> &Option<Vec<String>> {
        &self.require
    }
}

/// 持久内存（QEMU nvdimm → guest /dev/pmem0）；段缺省 = 关闭（argv 保持
/// 冻结基线）。size = 后端文件大小（缺省 "256M"）；require 按被测内核
/// 配置取舍（如 libnvdimm/nfit/nd_pmem 为 =m 时声明）。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmemComponent {
    pub(crate) enabled: Option<bool>,
    pub(crate) require: Option<Vec<String>>,
    pub(crate) stage: Option<Stage>,
    pub(crate) size: Option<StrVal>,
}

impl Component for PmemComponent {
    fn enabled(&self) -> Option<bool> {
        self.enabled
    }
    fn stage(&self) -> Option<Stage> {
        self.stage
    }
    fn require(&self) -> &Option<Vec<String>> {
        &self.require
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BusyboxSection {
    pub(crate) version: Option<StrVal>,
    pub(crate) release_repo: Option<StrVal>,
    pub(crate) dl_url: Option<StrVal>,
    pub(crate) force_source_build: Option<bool>,
}

pub(crate) fn parse_toml(path: &Path) -> anyhow::Result<VirtuosoToml> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("read {} failed", path.display()))?;
    toml::from_str(&raw).map_err(|e| {
        let mut msg = format!("{} parse failed (unknown key or invalid type): {e}", path.display());
        if raw.contains("[numa]") {
            msg.push_str("\n  The legacy [numa] section has moved to [components.numa] (enabled = true + nodes/memory_per_node)");
        }
        anyhow::anyhow!(msg)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ComponentPlan;

    fn parse(text: &str) -> anyhow::Result<VirtuosoToml> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("{e}"))
    }

    #[test]
    fn toml_globals_parse() {
        let cfg = parse(
            r#"
arch = "arm64"
timeout_secs = 60
smp = 4
auto_test = false
qemu_opts = ["-device vfio-pci,host=01:00.0"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.arch.as_ref().unwrap().0, "arm64");
        assert_eq!(cfg.timeout_secs.as_ref().unwrap().0, "60");
        assert_eq!(cfg.smp.as_ref().unwrap().0, "4");
        assert_eq!(cfg.auto_test, Some(false));
        assert_eq!(cfg.qemu_opts.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn components_parse_with_require_and_stage() {
        let cfg = parse(
            r#"
[components.tools_disk]
enabled = true

[components.agent]
enabled = true
require = ["virtio_console"]

[components.vfio]
enabled = true
devices = ["0000:01:00.0"]
require = ["vfio", "vfio_pci"]
stage = "boot"

[components.numa]
enabled = true
nodes = 2
memory_per_node = "1G"
"#,
        )
        .unwrap();
        let comps = cfg.components.as_ref().unwrap();
        assert_eq!(comps.tools_disk.as_ref().unwrap().enabled, Some(true));
        let agent = comps.agent.as_ref().unwrap();
        assert_eq!(agent.require.clone().unwrap(), vec!["virtio_console"]);
        let vfio = comps.vfio.as_ref().unwrap();
        assert_eq!(vfio.devices.clone().unwrap(), vec!["0000:01:00.0"]);
        assert_eq!(vfio.stage, Some(Stage::Boot));
        let numa = comps.numa.as_ref().unwrap();
        assert_eq!(numa.nodes.as_ref().unwrap().0, "2");
    }

    #[test]
    fn component_plan_unions_enabled_requires_in_schema_order() {
        // 顺序冻结：tools_disk → agent → vfio → numa → pmem；去重保首个；
        // 禁用组件的 require 不并入；stage=boot 分区到 boot_extra。
        let cfg = parse(
            r#"
[components.tools_disk]
enabled = true
require = ["virtio_blk"]

[components.agent]
enabled = true
require = ["virtio_console"]

[components.vfio]
enabled = false
require = ["virtio_blk"]
stage = "boot"

[components.numa]
enabled = true
require = ["crc64"]
stage = "boot"

[components.pmem]
enabled = true
require = ["nd_pmem"]
"#,
        )
        .unwrap();
        let comps = cfg.components.as_ref().unwrap();
        let mut plan = ComponentPlan::default();
        // 直接复用 Config::component_plan 的拼装逻辑太重（要全量 Config），
        // 这里镜像其调用顺序验证 push 语义。
        let td = comps.tools_disk.as_ref().unwrap();
        let ag = comps.agent.as_ref().unwrap();
        let nm = comps.numa.as_ref().unwrap();
        let pm = comps.pmem.as_ref().unwrap();
        plan.push(td.stage, &td.require);
        plan.push(ag.stage, &ag.require);
        // vfio disabled —— require 不并入
        plan.push(nm.stage, &nm.require);
        plan.push(pm.stage, &pm.require);
        assert_eq!(plan.runtime, ["virtio_blk", "virtio_console", "nd_pmem"]);
        assert_eq!(plan.boot_extra, ["crc64"]);
        assert_eq!(
            plan.all().cloned().collect::<Vec<_>>(),
            ["crc64", "virtio_blk", "virtio_console", "nd_pmem"]
        );
    }

    #[test]
    fn pmem_parses_with_size_and_defaults_disabled() {
        // 段缺省 = 关闭：size 访问器返回 None
        let cfg = parse("[components.pmem]\n").unwrap();
        let comps = cfg.components.as_ref().unwrap();
        assert!(comps.pmem.as_ref().unwrap().enabled.is_none());

        let cfg = parse(
            r#"
[components.pmem]
enabled = true
size = "512M"
require = ["libnvdimm", "nfit", "nd_pmem"]
"#,
        )
        .unwrap();
        let comps = cfg.components.as_ref().unwrap();
        let pm = comps.pmem.as_ref().unwrap();
        assert_eq!(pm.size.as_ref().unwrap().0, "512M");
        assert_eq!(
            pm.require.clone().unwrap(),
            vec!["libnvdimm", "nfit", "nd_pmem"]
        );
    }

    #[test]
    fn pmem_unknown_key_rejected() {
        assert!(parse("[components.pmem]\nenabled = true\nsizes = \"1G\"\n").is_err());
    }

    #[test]
    fn unknown_key_is_rejected_at_parse_time() {
        assert!(
            parse("arch = \"arm64\"\nshmp = 4\n").is_err(),
            "拼写错误的键必须在解析期报错"
        );
    }

    #[test]
    fn unknown_component_key_is_rejected() {
        assert!(parse("[components.agent]\nenableds = true\n").is_err());
    }

    #[test]
    fn wrong_type_is_rejected_at_parse_time() {
        assert!(parse("qemu_opts = [1, 2]").is_err());
        assert!(parse("arch = true").is_err());
        assert!(parse("[components.agent]\nstage = \"middle\"\n").is_err());
    }
}
