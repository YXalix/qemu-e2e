//! 全局标量访问器：arch / timeout / 内核供给路径 / qemu 透传。
//! 取值链统一走 `scalar`（env > toml > 缺省）与 `env_bool`（仅 auto_test）。

use std::path::PathBuf;

use anyhow::Context;

use crate::Arch;

use super::{env_bool, scalar, Config};

impl Config {
    /// 解析目标架构；无法解析时返回 None（诊断层告警）。
    pub fn arch(&self) -> Option<Arch> {
        self.arch_str()
            .and_then(|a| Arch::parse(&a))
            .or_else(Arch::host_default)
    }

    /// arch 原始字符串（诊断层呈现来源用）。
    pub fn arch_str(&self) -> Option<String> {
        scalar(self.tv(|t| &t.arch), "ARCH")
    }

    /// (KERNEL_PATH, 是否显式指定)。未指定时 = 项目根上一级（qemu-e2e 自动探测语义）。
    pub fn kernel_path(&self) -> anyhow::Result<(PathBuf, bool)> {
        match scalar(self.tv(|t| &t.kernel_path), "KERNEL_PATH") {
            Some(p) => Ok((PathBuf::from(p), true)),
            None => {
                let parent = self
                    .project_root
                    .parent()
                    .context("project root has no parent, cannot auto-detect KERNEL_PATH")?;
                Ok((parent.to_path_buf(), false))
            }
        }
    }

    /// kernel_image 覆盖（缺省 = 内核树内 arch 对应镜像）。
    pub fn kernel_image(&self) -> Option<String> {
        scalar(self.tv(|t| &t.kernel_image), "KERNEL_IMAGE")
    }

    /// QEMU_TIMEOUT 原始字符串（"0" 由 test 命令拒绝）。
    pub fn timeout_raw(&self) -> String {
        scalar(self.tv(|t| &t.timeout_secs), "QEMU_TIMEOUT").unwrap_or_else(|| "0".into())
    }

    /// AUTO_TEST 开关（缺省 true —— 常规启动即跑测试；env 覆盖 toml，
    /// env_bool 宽松语义：1/true/yes/on 为真）。
    pub fn auto_test(&self) -> bool {
        env_bool("AUTO_TEST")
            .unwrap_or_else(|| self.toml.as_ref().and_then(|t| t.auto_test).unwrap_or(true))
    }

    /// QEMU 二进制覆盖。
    pub fn qemu_override(&self) -> Option<String> {
        scalar(self.tv(|t| &t.qemu), "QEMU")
    }

    /// QEMU 透传参数（全局兜底）：`QEMU_OPTS` 环境变量（空白切分）优先，
    /// 否则 toml `qemu_opts` 数组。组件增量（vfio 设备等）不在此混入——
    /// 由装配点（cli::build_invocation）显式追加，全局兜底与组件增量分清。
    pub fn qemu_extra(&self) -> Vec<String> {
        match std::env::var("QEMU_OPTS") {
            Ok(s) if !s.trim().is_empty() => s.split_whitespace().map(str::to_string).collect(),
            _ => self
                .toml
                .as_ref()
                .and_then(|t| t.qemu_opts.as_ref())
                .map(|list| list.to_vec())
                .unwrap_or_default(),
        }
    }
}
