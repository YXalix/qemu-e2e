//! 内存量字符串解析（NUMA 拓扑与 firecracker 共用同一张合法单位表）。

/// 内存单位后缀；`Bare` = 无单位裸数字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemUnit {
    G,
    M,
    Bare,
}

impl MemUnit {
    /// 数值 × 单位 → 规范字符串（"2G" / "512M" / "1024"）。
    pub fn render(self, n: u64) -> String {
        match self {
            MemUnit::G => format!("{n}G"),
            MemUnit::M => format!("{n}M"),
            MemUnit::Bare => n.to_string(),
        }
    }

    /// 数值 → MiB（G = ×1024；M 与裸数字原值）。
    pub fn to_mib(self, n: u64) -> u64 {
        match self {
            MemUnit::G => n * 1024,
            MemUnit::M | MemUnit::Bare => n,
        }
    }
}

/// 解析 "1G" / "512M" / "1024"（单位大小写不敏感）→ (数值, 单位)。
/// 非数字前缀或单位不在 G/M/裸数字表内时报错。
pub fn parse_memory(s: &str) -> Result<(u64, MemUnit), String> {
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n: u64 = s[..digits]
        .parse()
        .map_err(|_| format!("内存值无法解析: {s}"))?;
    let unit = match s[digits..].trim() {
        "" => MemUnit::Bare,
        "G" | "g" => MemUnit::G,
        "M" | "m" => MemUnit::M,
        other => return Err(format!("内存单位不支持: {other}")),
    };
    Ok((n, unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_memory_units() {
        assert_eq!(parse_memory("2G").unwrap(), (2, MemUnit::G));
        assert_eq!(parse_memory("512m").unwrap(), (512, MemUnit::M));
        assert_eq!(parse_memory("1024").unwrap(), (1024, MemUnit::Bare));
        assert_eq!(MemUnit::G.render(2), "2G");
        assert_eq!(MemUnit::G.to_mib(2), 2048);
        assert!(parse_memory("1X").is_err());
        assert!(parse_memory("x1").is_err());
    }
}
