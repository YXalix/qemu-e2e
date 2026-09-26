//! 运行工件与分诊报告（runs 领域）。
//!
//! 职责边界：
//! - 本模块只做 **IO 与呈现**：run 目录（`rundir`）、输出泵、报告落盘/回读、
//!   triage/cluster 展示（`render`）；
//! - **语义**在 judge（标记协议解析 + verdict 判定 + schema），本模块不重复实现；
//! - verdict.json 的 schema 单点定义在 `judge::report::VerdictReport`（此处
//!   re-export）；退出码语义单点在 `judge::exit`。正常收尾与 Ctrl-C 兜底
//!   共用同一构造路径。

pub mod render;
pub mod rundir;

pub use judge::report::RunMeta;
pub use render::{run_cluster, run_triage};
pub use rundir::{create_run_dir, finalize_run, prune, pump_child, RunDir, RUNS_KEEP};
