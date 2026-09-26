//! 文件系统与进程环境工具：PATH 查找、可执行位、ELF 魔数。

use std::path::{Path, PathBuf};

/// 定位可执行文件的完整路径（不在 PATH 返回 None）。
pub(crate) fn which_path(bin: &str) -> Option<PathBuf> {
    if bin.contains('/') {
        return Path::new(bin).is_file().then(|| PathBuf::from(bin));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(bin))
            .find(|p| p.is_file())
    })
}

/// 定位可执行文件是否在 PATH（verify 用）。
pub(crate) fn which(bin: &str) -> bool {
    which_path(bin).is_some()
}

/// 置 0o755（下载产物 / VM 内 init / 测试二进制统一使用）。
pub(crate) fn set_executable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
}

/// 读取文件前 4 字节（ELF 魔数校验的共用原语）。
fn read_magic(p: &Path) -> std::io::Result<[u8; 4]> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(p)?.read_exact(&mut magic)?;
    Ok(magic)
}

/// 文件是否以 \x7fELF 魔数开头（读取失败一律视为否）。
pub(crate) fn is_elf(p: &Path) -> bool {
    read_magic(p)
        .map(|m| m == [0x7f, b'E', b'L', b'F'])
        .unwrap_or(false)
}
