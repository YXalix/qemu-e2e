//! 退出码语义 —— 唯一的语义表（冻结：0=通过、124=超时、137 归一为 124、
//! 130=中断；其余非零=失败）。`judge` 的对账、host 侧收割与 verdict.json 的
//! `exit_semantics` 字段都从这里取值，不允许各自再写一份映射。

/// 超时退出码（与 `timeout --signal=KILL` 的 124 语义一致）。
pub const EXIT_TIMEOUT: i32 = 124;
/// Ctrl-C / 信号中断退出码。
pub const EXIT_INTERRUPTED: i32 = 130;

/// 退出码归一：137（SIGKILL）按超时语义归一为 124；无退出码（信号死亡）
/// 归一为 130；其余保留真实码（xtask 保留 make 折叠前的原始语义）。
pub fn normalize(code: Option<i32>) -> i32 {
    match code {
        Some(124) | Some(137) => EXIT_TIMEOUT,
        Some(n) => n,
        None => EXIT_INTERRUPTED,
    }
}

/// 退出码 → 语义标签（verdict.json 的 `exit_semantics` 字段）。
pub fn semantics(code: Option<i32>) -> &'static str {
    match code {
        Some(0) => "ok",
        Some(124) => "timeout",
        Some(130) => "interrupted",
        Some(_) => "error",
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_preserves_real_codes() {
        assert_eq!(normalize(Some(0)), 0);
        assert_eq!(normalize(Some(1)), 1);
        assert_eq!(normalize(Some(124)), 124);
        assert_eq!(normalize(Some(137)), 124, "SIGKILL 按超时语义归一");
        assert_eq!(normalize(None), 130);
    }

    #[test]
    fn semantics_labels() {
        assert_eq!(semantics(Some(0)), "ok");
        assert_eq!(semantics(Some(124)), "timeout");
        assert_eq!(semantics(Some(130)), "interrupted");
        assert_eq!(semantics(Some(1)), "error");
        assert_eq!(semantics(None), "unknown");
    }
}
