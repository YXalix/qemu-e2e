//! POSIX shell 单参引用（shlex.quote 语义）：安全字符原样，其余整体单引号
//! 包裹。用于把 argv 渲染成可直接复制执行的一行命令（launcher 的
//! `command_line`），以及把路径/URL/ref 安全嵌入容器 `sh -c` 脚本（forge）。

/// 安全字符集：字母数字与 shell 元字符之外的普通文件名成分——单参不含
/// 空白/引号/glob 字符时原样输出即等价。
fn is_safe(arg: &str) -> bool {
    !arg.is_empty()
        && arg
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'@' | b'%' | b'+' | b','))
}

/// POSIX shell 单参引用：安全字符原样，其余整体单引号包裹（内嵌单引号转义）。
pub fn quote(arg: &str) -> String {
    if is_safe(arg) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_safe_chars_verbatim_spaces_quoted() {
        assert_eq!(quote("virt"), "virt");
        assert_eq!(quote("file=a.img,format=raw"), "file=a.img,format=raw");
        assert_eq!(
            quote("https://gh.example.com/x/y.git"),
            "https://gh.example.com/x/y.git"
        );
        assert_eq!(quote("/tmp/a b.img"), "'/tmp/a b.img'");
        assert_eq!(quote("a'b"), "'a'\\''b'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("*junk*"), "'*junk*'");
    }
}
