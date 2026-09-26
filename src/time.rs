//! 时间工具：无 chrono 依赖的 UTC 格式化。

/// 当前 Unix 毫秒（时钟回拨时退化为 0）。
pub(crate) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Unix 毫秒 → "YYYY-MM-DD HH:MM:SS UTC"（civil_from_days 算法）。
pub(crate) fn format_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// 公历日数 → (年, 月, 日)（Howard Hinnant 的 civil_from_days 算法）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 历法锚点：纪元边界、平年/闰年切换（civil_from_days 只在这几类
    /// 规则上出错，锚点钉死即够）。
    #[test]
    fn format_utc_calendar_anchors() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_000), "1970-01-01 00:00:01 UTC");
        assert_eq!(format_utc(86_399_999), "1970-01-01 23:59:59 UTC");
        // 平年→闰年：2024-02-28 与 02-29
        assert_eq!(format_utc(1_709_107_200_000), "2024-02-28 08:00:00 UTC");
        assert_eq!(format_utc(1_709_193_600_000), "2024-02-29 08:00:00 UTC");
        // 闰年→平年回落：2024-03-01
        assert_eq!(format_utc(1_709_280_000_000), "2024-03-01 08:00:00 UTC");
        // 年末进位与 400 年闰 2000
        assert_eq!(format_utc(1_735_689_599_999), "2024-12-31 23:59:59 UTC");
        assert_eq!(format_utc(946_684_800_000), "2000-01-01 00:00:00 UTC");
    }

    #[test]
    fn unix_ms_is_epoch_scale() {
        let now = unix_ms();
        assert!(now > 1_700_000_000_000); // 2023-11 之后（装机即真）
    }
}
