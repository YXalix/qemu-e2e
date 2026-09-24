//! 用例构建脚本：把 `c/*.c` 编入本用例二进制（C over Rust）。
//!
//! 编译器选择是显式单路径：`CC_<TRIPLE>`（UPPER_SNAKE，builder::cross 注入
//! 的 zig cc 包装）→ `CC_<triple 连字符>` → `TARGET_CC` → `CC` → 交回 cc
//! crate 宿主缺省。**交叉构建（TARGET != HOST）且没有任何 CC 接管时
//! fail-fast**——防止 cc crate 静默落到 host cc，编出 glibc 目标码混链
//! musl libc（ABI 风险），这正是统一 zig 的理由。
//!
//! 顺带把解析出的编译命令写成 workspace 根的 `compile_commands.json`
//! （gitignore）：clangd 据此供 C 侧 LSP；rust-analyzer 触发的 host check
//! 生成 host 版，恰是 IDE 想要的形态。

use std::env;
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.parent().expect("crate 目录必有父目录");
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let cross = !target.is_empty() && target != host;

    let compiler = compiler_env(&target);
    if cross && compiler.is_none() {
        let upper = target.to_uppercase().replace('-', "_");
        eprintln!("test-example build.rs: 交叉构建 {target} 需要 C 编译器接管（CC_{upper}）");
        eprintln!("  正常路径：`virtuoso build`（builder 注入 zig cc 包装）");
        eprintln!("  自带工具链：CC_{upper}=<cross-cc>");
        exit(1);
    }

    let mut build = cc::Build::new();
    if let Some(cc_bin) = &compiler {
        build.compiler(cc_bin);
    }
    build
        .warnings(true)
        .include(manifest.join("c"))
        .include(workspace.join("framework").join("include"));
    let sources = c_sources(&manifest.join("c"));
    for src in &sources {
        println!("cargo:rerun-if-changed={}", src.display());
        build.file(src);
    }
    println!(
        "cargo:rerun-if-changed={}",
        workspace
            .join("framework")
            .join("include")
            .join("testfw.h")
            .display()
    );
    // 对象直链（rustc-link-arg-bins），不走 cc 的 compile()：交叉场景下
    // 宿主 ar/ranlib 给 ELF 目码建不了符号索引（macOS 实测 undefined
    // symbol），命令行对象文件则被链接器无条件接纳，无此问题。
    let objs = build.compile_intermediates();
    for obj in &objs {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    }

    write_compile_commands(workspace, &manifest, compiler.as_deref());
}

/// cc crate 同序的编译器环境探测（显式化，不依赖 cc 内部解析细节）：
/// 目标专属键优先，再退全局键。
fn compiler_env(target: &str) -> Option<PathBuf> {
    [
        format!("CC_{}", target),
        format!("CC_{}", target.to_uppercase().replace('-', "_")),
        "TARGET_CC".to_string(),
        "CC".to_string(),
    ]
    .into_iter()
    .find_map(|k| env::var_os(&k).map(PathBuf::from))
}

fn c_sources(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "c").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// 把本次 C 编译的条目写成 workspace 根 `compile_commands.json`。
/// 条目是给 clangd 的最小形态（编译器 + include + -std），不必与 cc
/// crate 的完整 argv 逐字一致。
fn write_compile_commands(workspace: &Path, manifest: &Path, compiler: Option<&Path>) {
    let compiler = compiler
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "cc".to_string());
    let framework_include = manifest
        .parent()
        .expect("crate 必有父目录")
        .join("framework")
        .join("include");

    let mut sources = c_sources(&manifest.join("c"));
    // header 也给一条条目，clangd 悬停头文件时有落点
    sources.push(framework_include.join("testfw.h"));

    let entries: Vec<String> = sources
        .iter()
        .filter(|f| f.exists())
        .map(|file| {
            let args = [
                compiler.clone(),
                "-std=c11".to_string(),
                format!("-I{}", manifest.join("c").display()),
                format!("-I{}", framework_include.display()),
                "-Wall".to_string(),
                "-c".to_string(),
                file.display().to_string(),
                "-o".to_string(),
                "/dev/null".to_string(),
            ];
            let args_json: Vec<String> = args.iter().map(|a| json_str(a.as_str())).collect();
            format!(
                "{{\"directory\": {}, \"arguments\": [{}], \"file\": {}}}",
                json_str(&manifest.display().to_string()),
                args_json.join(", "),
                json_str(&file.display().to_string())
            )
        })
        .collect();
    let out = workspace.join("compile_commands.json");
    let _ = std::fs::write(&out, format!("[{}]\n", entries.join(",\n")));
}

/// 极简 JSON 字符串转义（路径与编译器命令不含控制字符，够用）。
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
