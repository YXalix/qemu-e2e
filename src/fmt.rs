//! 人类可读格式化。

/// ls -lh 风格的大小（36.7M / 1.5G）。
pub(crate) fn human_size_ls(bytes: u64) -> String {
    let (n, unit) = if bytes >= 1 << 30 {
        (bytes as f64 / (1 << 30) as f64, "G")
    } else if bytes >= 1 << 20 {
        (bytes as f64 / (1 << 20) as f64, "M")
    } else if bytes >= 1 << 10 {
        (bytes as f64 / (1 << 10) as f64, "K")
    } else {
        return format!("{bytes}B");
    };
    if n >= 10.0 {
        format!("{n:.0}{unit}")
    } else {
        format!("{n:.1}{unit}")
    }
}
