//! 运行工件（runs 领域）。
//!
//! 职责边界：
//! - 本模块只做 **IO**：run 目录（`rundir`）、输出泵、verdict.json 落盘；
//!   判定呈现单点在 `test` 收尾行 + `verdict.json`（机读唯一面），无第二呈现命令；
//! - **语义**在 judge（标记协议解析 + verdict 判定 + schema），本模块不重复实现；
//! - verdict.json 的 schema 单点定义在 `judge::report::VerdictReport`（此处
//!   re-export）；退出码语义单点在 `judge::exit`。正常收尾与 Ctrl-C 兜底
//!   共用同一构造路径。

pub mod rundir;

pub use judge::report::RunMeta;
pub use rundir::{create_run_dir, finalize_run, prune, pump_child, RunDir, RUNS_KEEP};
