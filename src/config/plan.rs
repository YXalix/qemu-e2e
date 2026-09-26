//! ComponentPlan：启用组件的 KO 依赖并集，按 stage 分区。
//! builder 消费该计划生成 initramfs 基础集追加与 rootfs modules.conf。

/// 启用组件的 KO 依赖并集：`boot_extra` 追加在 modules-boot.conf 冻结
/// 基础集之后（initramfs 阶段加载），`runtime` 生成 rootfs 的
/// /lib/modules/modules.conf（switch_root 后加载）。条目 = conf 行
/// `"<module> [key=val ...]"`，首 token 是模块名（去重键）。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ComponentPlan {
    pub boot_extra: Vec<String>,
    pub runtime: Vec<String>,
}

impl ComponentPlan {
    pub(crate) fn push(&mut self, stage: Option<super::schema::Stage>, require: &Option<Vec<String>>) {
        let Some(lines) = require else { return };
        let boot = stage == Some(super::schema::Stage::Boot);
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let name = line.split_whitespace().next().unwrap_or_default();
            let slot = if boot {
                &mut self.boot_extra
            } else {
                &mut self.runtime
            };
            if slot
                .iter()
                .any(|e| e.split_whitespace().next().unwrap_or_default() == name)
            {
                continue;
            }
            slot.push(line.to_string());
        }
    }

    /// 全部条目（boot_extra 在前）—— verify 的模块存在性检查输入。
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.boot_extra.iter().chain(self.runtime.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::ComponentPlan;
    use super::super::schema::Stage;

    #[test]
    fn component_plan_dedups_by_module_name_keeping_first_args() {
        let mut plan = ComponentPlan::default();
        plan.push(
            None,
            &Some(vec!["nvme-core poll_queues=2".into(), "nvme-core".into()]),
        );
        assert_eq!(plan.runtime, ["nvme-core poll_queues=2"]);
        plan.push(Some(Stage::Boot), &Some(vec!["nvme-core".into()]));
        assert_eq!(plan.boot_extra, ["nvme-core"], "去重只在同 stage 分区内");
    }
}
