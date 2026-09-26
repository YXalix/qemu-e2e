//! 终端呈现原语：TTY 感知颜色与状态图标。
//!
//! 约定：颜色只修饰不承载语义（非 TTY / NO_COLOR 自动退化纯文本）；
//! 状态图标 ✓ / ✗ 与 [PASS]/[FAIL] 同义。

/// stdout 是否着色（TERM 存在且非 dumb，且无 NO_COLOR；仅影响颜色不影响判定）。
pub(crate) fn tty() -> bool {
    std::env::var_os("TERM")
        .map(|t| t != "dumb")
        .unwrap_or(false)
        && std::env::var_os("NO_COLOR").is_none()
}

/// ANSI 包裹（非 TTY 原样返回）。
pub(crate) fn paint(code: &str, s: &str) -> String {
    if tty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// 粗体。
pub(crate) fn bold(s: &str) -> String {
    paint("1", s)
}

/// 绿（通过）。
pub(crate) fn green(s: &str) -> String {
    paint("0;32", s)
}

/// 红（失败）。
pub(crate) fn red(s: &str) -> String {
    paint("0;31", s)
}

/// 状态图标（按最严重级别取）：✓ / ✗。
pub(crate) fn icon(level: Icon) -> String {
    match level {
        Icon::Pass => green("✓"),
        Icon::Fail => red("✗"),
    }
}

pub(crate) enum Icon {
    Pass,
    Fail,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_degrades_without_tty() {
        // 测试进程 TERM 可能存在；两个分支都要产出合法字符串
        let s = paint("1", "x");
        assert!(s == "x" || s == "\x1b[1mx\x1b[0m");
    }
}
