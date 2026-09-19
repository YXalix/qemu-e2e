//! 文件系统与进程环境工具：PATH 查找、可执行位、ELF 魔数。

use std::path::Path;

/// 定位可执行文件是否在 PATH（verify 用）。
pub fn which(bin: &str) -> bool {
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

/// 置 0o755（下载产物 / VM 内 init / 测试二进制统一使用）。
pub fn set_executable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
}

/// 读取文件前 4 字节（ELF 魔数校验的共用原语）。
pub fn read_magic(p: &Path) -> std::io::Result<[u8; 4]> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(p)?.read_exact(&mut magic)?;
    Ok(magic)
}

/// 文件是否以 \x7fELF 魔数开头（读取失败一律视为否）。
pub fn is_elf(p: &Path) -> bool {
    read_magic(p).map(|m| m == [0x7f, b'E', b'L', b'F']).unwrap_or(false)
}
