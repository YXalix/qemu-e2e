//! 运行工件与分诊报告（runs 领域）。
//!
//! 职责边界：
//! - 本模块只做 **IO 与呈现**：run 目录（`rundir`）、输出泵、报告落盘/回读、
//!   triage/runs/cluster/suggest/replay 展示（`render`）；
//! - **语义**在 judge（标记协议解析 + verdict 判定 + schema），本模块不重复实现；
//! - verdict.json 的 schema 单点定义在 `judge::report::VerdictReport`（此处
//!   re-export）；退出码语义单点在 `judge::exit`。正常收尾与 Ctrl-C 兜底
//!   共用同一构造路径。

pub mod render;
pub mod rundir;

pub use rundir::{RunDir, RUNS_KEEP, create_run_dir, finalize_run, latest_run, load_verdict_or_parse, pump_child, prune};
pub use judge::report::RunMeta;
pub use render::{run_cluster, run_replay, run_runs, run_suggest, run_triage};
