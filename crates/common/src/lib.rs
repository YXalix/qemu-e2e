//! common — 基础层：架构矩阵与跨 crate 通用工具（零依赖）。
//!
//! 所有上层 crate（builder / launcher / judge / guardian / tracker / cli）
//! 都可以依赖本 crate，它不依赖任何 workspace 成员。`Arch` 是三架构矩阵的
//! 唯一事实来源（launcher 对外 re-export 以保持启动 DSL 的自洽入口）。

pub mod arch;
pub mod fmt;
pub mod fsutil;
pub mod platform;
pub mod time;
pub mod units;

pub use arch::{Arch, ALL_ARCHES};
pub use platform::HostOs;
