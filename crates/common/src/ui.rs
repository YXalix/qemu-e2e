//! 终端呈现原语：TTY 感知颜色、状态图标、统一 tag。
//!
//! 约定：颜色只修饰不承载语义（非 TTY / NO_COLOR 自动退化纯文本）；
//! 状态图标 ✓ / ✗ / ! 与 [PASS]/[FAIL]/[WARN] 同义；`[TAG]` 前缀一 tag
//! 一语义：[RUN] 工件与轮次、[LAUNCH] 启动命令、[DOCTOR] 体检、
//! [REPLAY] 各归其位。

/// stdout 是否着色（TERM 存在且非 dumb，且无 NO_COLOR；仅影响颜色不影响判定）。
pub fn tty() -> bool {
    std::env::var_os("TERM")
        .map(|t| t != "dumb")
        .unwrap_or(false)
        && std::env::var_os("NO_COLOR").is_none()
}

/// ANSI 包裹（非 TTY 原样返回）。
pub fn paint(code: &str, s: &str) -> String {
    if tty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// 粗体。
pub fn bold(s: &str) -> String {
    paint("1", s)
}

/// 绿（通过）。
pub fn green(s: &str) -> String {
    paint("0;32", s)
}

/// 红（失败）。
pub fn red(s: &str) -> String {
    paint("0;31", s)
}

/// 黄（告警）。
pub fn yellow(s: &str) -> String {
    paint("0;33", s)
}

/// 青（信息）。
pub fn cyan(s: &str) -> String {
    paint("0;36", s)
}

/// 状态图标（按最严重级别取）：✓ / ✗ / !。
pub fn icon(level: Icon) -> String {
    match level {
        Icon::Pass => green("✓"),
        Icon::Fail => red("✗"),
        Icon::Warn => yellow("!"),
    }
}

pub enum Icon {
    Pass,
    Fail,
    Warn,
}

/// `[TAG]` 前缀（粗体；非 TTY 纯文本）。
pub fn tag(name: &str) -> String {
    bold(&format!("[{name}]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_degrades_without_tty() {
        // 测试进程 TERM 可能存在；两个分支都要产出合法字符串
        let s = paint("1", "x");
        assert!(s == "x" || s == "\x1b[1mx\x1b[0m");
        assert!(tag("RUN") == "[RUN]" || tag("RUN") == "\x1b[1m[RUN]\x1b[0m");
    }
}
