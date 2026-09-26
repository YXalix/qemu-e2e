//! 中断退出码常量（与 shell 信号语义一致）。
//!
//! guardian（Ctrl-C 守护）不依赖 judge，但需要同一常量——故钉在这里，
//! judge::exit（唯一语义表）re-export 对外。

/// Ctrl-C / 信号中断退出码。
pub(crate) const EXIT_INTERRUPTED: i32 = 130;
