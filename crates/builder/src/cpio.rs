//! cpio newc 格式原生写入 —— `find | cpio -H newc | gzip` 管道的 Rust 接管。
//!
//! 单路径跨平台（Linux/macOS 同码），消灭 GNU cpio 的 `--null` 平台差异与
//! bash/find/gzip 外部依赖。写入约定与原管道对齐：
//! - 条目名带 `./` 前缀（内核 initramfs 解包器以 cwd=/ 消费，形态同 find 输出）；
//! - 符号链接存 target 文本（filesize = target 长度，mode S_IFLNK，权限位恒
//!   0777——Linux symlink 恒 0777 而 macOS 应用 umask，symlink 权限本身无语义，
//!   归一化才保证跨宿主字节一致）；
//! - mtime/ino 无语义价值，恒写 0/递增序号 → **产物按内容确定性可复现**；
//! - 目录条目 nlink=2、文件/链接 nlink=1（内核解包器不消费 nlink）。

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::Context;

/// 把 `dir` 整树打包为 gzip 压缩的 newc cpio，写入 `out`。
pub fn pack_dir_gzip(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let mut entries: Vec<(String, std::fs::Metadata)> = Vec::new();
    collect_entries(dir, dir, &mut entries)?;
    // 确定性：路径排序（GNU find 为 readdir 序，内容等价、顺序不构成契约）
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let out_file = File::create(out).with_context(|| format!("create {} failed", out.display()))?;
    let mut gz = flate2::write::GzEncoder::new(out_file, flate2::Compression::best());
    write_stream(&entries, dir, &mut gz)?;
    gz.finish().with_context(|| "gzip finish failed")?;
    Ok(())
}

/// 递归收集 `root` 下全部条目（含符号链接本体，不跟随），名为相对 root 的
/// `./`-前缀路径（`.` 根条目本身跳过，与 find 输出差异不影响解包）。
fn collect_entries(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, std::fs::Metadata)>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("read {} failed", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .with_context(|| format!("stat {} failed", path.display()))?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = meta.is_dir();
        out.push((format!("./{rel}"), meta));
        if is_dir {
            collect_entries(root, &path, out)?;
        }
    }
    Ok(())
}

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

/// 4 字节对齐（newc 的头/数据段边界规则）。
fn align4(n: usize) -> usize {
    n + (4 - n % 4) % 4
}

/// 逐条目写 newc 流 + TRAILER!!!。
fn write_stream<W: Write>(
    entries: &[(String, std::fs::Metadata)],
    root: &Path,
    w: &mut W,
) -> anyhow::Result<()> {
    for (i, (name, meta)) in entries.iter().enumerate() {
        use std::os::unix::fs::MetadataExt;
        let ftype = meta.mode() & S_IFMT;
        let (mode, size) = match ftype {
            S_IFDIR => (meta.mode() & 0o7777 | S_IFDIR, 0u64),
            S_IFLNK => (0o777 | S_IFLNK, meta.len()),
            _ => (meta.mode() & 0o7777 | S_IFREG, meta.len()),
        };
        let nlink: u64 = if ftype == S_IFDIR { 2 } else { 1 };
        write_header(w, name, i as u64 + 1, mode, nlink, size)?;
        let rel = Path::new(name).strip_prefix(".").unwrap_or(Path::new(name));
        match ftype {
            S_IFLNK => {
                // 符号链接：数据段 = target 路径文本
                let target = std::fs::read_link(root.join(rel))
                    .with_context(|| format!("readlink {name} failed"))?;
                write_data(w, target.as_os_str().as_encoded_bytes())?;
            }
            S_IFREG => {
                let data =
                    std::fs::read(root.join(rel)).with_context(|| format!("read {name} failed"))?;
                write_data(w, &data)?;
            }
            _ => {} // 目录无数据段
        }
    }
    write_header(w, "TRAILER!!!", 0, 0, 1, 0)?;
    // 尾部补齐到 512（惯例；内核解包器遇 TRAILER!!! 即停）
    let pad = 512 - align4(110 + "TRAILER!!!".len() + 1);
    w.write_all(&vec![0u8; pad])?;
    Ok(())
}

/// newc 头（110 字节 ASCII）：magic + 13 个 8 位十六进制字段。
/// uid/gid/mtime/设备号恒 0（内核解包器以 root 身份消费，不读这些字段）。
fn write_header<W: Write>(
    w: &mut W,
    name: &str,
    ino: u64,
    mode: u32,
    nlink: u64,
    size: u64,
) -> anyhow::Result<()> {
    let namesize = name.len() + 1; // 含 NUL
    let fields = [
        ino,
        mode as u64,
        0,
        0,
        nlink,
        0, // mtime
        size,
        0,
        0, // dev major/minor
        0,
        0, // rdev major/minor
        namesize as u64,
        0, // check
    ];
    let mut hdr = String::with_capacity(110);
    hdr.push_str("070701");
    for f in fields {
        hdr.push_str(&format!("{f:08x}"));
    }
    w.write_all(hdr.as_bytes())?;
    w.write_all(name.as_bytes())?;
    w.write_all(&[0])?;
    // 头 + 名字补齐到 4 字节边界（newc 对齐规则）
    let pad = (4 - (110 + namesize) % 4) % 4;
    w.write_all(&vec![0u8; pad])?;
    Ok(())
}

fn write_data<W: Write>(w: &mut W, data: &[u8]) -> anyhow::Result<()> {
    w.write_all(data)?;
    let pad = (4 - data.len() % 4) % 4;
    w.write_all(&vec![0u8; pad])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mk_tree(dir: &Path) {
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("lib/modules")).unwrap();
        std::fs::write(dir.join("init"), b"#!/bin/sh\n").unwrap();
        set_mode(&dir.join("init"), 0o755);
        std::fs::write(dir.join("bin/busybox"), b"\x7fELF....").unwrap();
        set_mode(&dir.join("bin/busybox"), 0o755);
        std::os::unix::fs::symlink("busybox", dir.join("bin/sh")).unwrap();
        std::fs::write(dir.join("lib/modules/modules.conf"), "ext4\n").unwrap();
    }

    fn set_mode(p: &Path, mode: u32) {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// 解析出 (name, mode, size, data) 序列（对齐规则与写入侧镜像）。
    fn parse(cpio: &[u8]) -> Vec<(String, u32, u64, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        loop {
            let hdr = &cpio[i..i + 110];
            assert_eq!(&hdr[..6], b"070701", "newc magic");
            let hex = |k: usize| {
                u64::from_str_radix(
                    std::str::from_utf8(&hdr[6 + k * 8..6 + (k + 1) * 8]).unwrap(),
                    16,
                )
                .unwrap()
            };
            let mode = hex(1) as u32;
            let size = hex(6);
            let namesize = hex(11) as usize - 1; // 去 NUL
            let name = String::from_utf8(cpio[i + 110..i + 110 + namesize].to_vec()).unwrap();
            if name == "TRAILER!!!" {
                break;
            }
            let data_start = i + align4(110 + namesize + 1);
            let data = cpio[data_start..data_start + size as usize].to_vec();
            out.push((name, mode, size, data));
            i = data_start + align4(size as usize);
        }
        out
    }

    #[test]
    fn roundtrip_newc_entries() {
        let dir = std::env::temp_dir().join(format!("cpio-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        mk_tree(&dir);
        let out = dir.join("initrd.img");
        pack_dir_gzip(&dir, &out).unwrap();

        // gzip 魔数
        let raw = std::fs::read(&out).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b]);

        // 解压后逐条断言
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut cpio = Vec::new();
        std::io::Read::read_to_end(&mut dec, &mut cpio).unwrap();
        let entries = parse(&cpio);
        let get = |n: &str| entries.iter().find(|(name, ..)| name == n).unwrap();
        let (_, mode, _, data) = get("./init");
        assert_eq!(*mode, 0o100755);
        assert_eq!(data, b"#!/bin/sh\n");
        let (_, mode, _, data) = get("./bin/sh");
        assert_eq!(*mode, 0o120777);
        assert_eq!(data, b"busybox");
        let (_, mode, ..) = get("./lib/modules");
        assert_eq!(*mode, 0o040755);
        for expect in ["./bin", "./bin/busybox", "./init", "./lib", "./lib/modules"] {
            assert!(
                entries.iter().any(|(name, ..)| name == expect),
                "缺 {expect}"
            );
        }
    }

    /// 确定性：同树两次打包字节一致（mtime 恒 0 + 路径排序）。
    /// 输出放在树外——写进树内会被下一次收集吞进去。
    #[test]
    fn pack_is_deterministic() {
        let base = std::env::temp_dir().join(format!("cpio-det-{}", std::process::id()));
        let dir = base.join("tree");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        mk_tree(&dir);
        let a = base.join("a.img");
        let b = base.join("b.img");
        pack_dir_gzip(&dir, &a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100)); // 跨秒界
        set_mode(&dir.join("init"), 0o755); // 触发 mtime 变化
        pack_dir_gzip(&dir, &b).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    }

    fn align4(n: usize) -> usize {
        n + (4 - n % 4) % 4
    }
}
