//! coda-build — 用例 build.rs 的共享实现（coda 测试框架的配套 build-dep）。
//!
//! 每个用例 crate 的 build.rs 固定一行：
//!
//! ```ignore
//! fn main() { coda_build::run() }
//! ```
//!
//! 职责：把用例 `c/*.c` 编入本用例二进制（C over Rust），并把解析出的
//! 编译命令写成 crate 根的 `compile_commands.json`（gitignore）供 clangd。
//! `c/` 缺失或为空时编入一个空桩 `run_c_tests`——C 段符号恒存在，
//! `coda::run_and_exit` 无条件调用也不缺链。
//!
//! 编译器选择是显式单路径：`CC_<TRIPLE>`（builder::cross 注入的 zig cc
//! 包装）→ `CC_<triple 连字符>` → `TARGET_CC` → `CC` → 交回 cc crate 宿主
//! 缺省。**交叉构建（TARGET != HOST）且没有任何 CC 接管时 fail-fast**——
//! 防 cc crate 静默落到 host cc，编出 glibc 目标码混链 musl libc（ABI
//! 风险），这正是统一 zig 的理由。
//!
//! 约定：用例 crate 是 `infra/testcases/` 的直接子目录（coda 在
//! `../coda/include`）。对象直链（rustc-link-arg-bins），不走 cc 的
//! compile()：交叉场景下宿主 ar/ranlib 给 ELF 目码建不了符号索引（macOS
//! 实测 undefined symbol），命令行对象文件则被链接器无条件接纳，无此问题。

use std::env;
use std::path::{Path, PathBuf};
use std::process::exit;

/// 用例 build.rs 入口（读 CARGO_MANIFEST_DIR / TARGET / HOST 等 build
/// script 环境变量）。
pub fn run() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest
        .parent()
        .expect("crate 目录必有父目录")
        .to_path_buf();
    let pkg = env::var("CARGO_PKG_NAME").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let cross = !target.is_empty() && target != host;
    let c_dir = manifest.join("c");
    let coda_include = workspace.join("coda").join("include");

    let compiler = compiler_env(&target);
    if cross && compiler.is_none() {
        let upper = target.to_uppercase().replace('-', "_");
        eprintln!("{pkg} build.rs: 交叉构建 {target} 需要 C 编译器接管（CC_{upper}）");
        eprintln!("  正常路径：`virtuoso build`（builder 注入 zig cc 包装）");
        eprintln!("  自带工具链：CC_{upper}=<cross-cc>");
        exit(1);
    }

    let mut build = cc::Build::new();
    if let Some(cc_bin) = &compiler {
        build.compiler(cc_bin);
    }
    build.warnings(true).include(&c_dir).include(&coda_include);

    let sources = c_sources(&c_dir);
    // 目录级 rerun：覆盖增/删/改（逐文件粒度反而漏掉"新增 .c 不重编译"）
    println!("cargo:rerun-if-changed={}", c_dir.display());
    println!(
        "cargo:rerun-if-changed={}",
        coda_include.join("coda.h").display()
    );
    for src in &sources {
        build.file(src);
    }
    if sources.is_empty() {
        // 空桩：无 C 段的用例由桩兜住 run_c_tests 符号
        let stub = PathBuf::from(env::var("OUT_DIR").unwrap()).join("coda_c_stub.c");
        std::fs::write(&stub, "void run_c_tests(void) {}\n").expect("写 OUT_DIR 失败");
        build.file(&stub);
    }
    // 对象直链：见模块 doc 的 ar/ranlib 理由
    for obj in &build.compile_intermediates() {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    }

    write_compile_commands(&manifest, &c_dir, &coda_include, compiler.as_deref());
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

/// 把本次 C 编译的条目写成 crate 根 `compile_commands.json`。
/// 条目是给 clangd 的最小形态（编译器 + include + -std），不必与 cc
/// crate 的完整 argv 逐字一致。clangd 沿源文件祖先目录取最近的 CDB，
/// 按 crate 天然路由，多用例并行构建各写各的、无共享写点。
/// rust-analyzer 触发的 host check 生成 host 版，恰是 IDE 想要的形态；
/// 旧版写 workspace 根的共享 CDB（后建 crate 整文件覆写先建 crate 的
/// 条目）已退役，这里清掉存量。
fn write_compile_commands(
    manifest: &Path,
    c_dir: &Path,
    coda_include: &Path,
    compiler: Option<&Path>,
) {
    let workspace = manifest.parent().expect("crate 必有父目录");
    let _ = std::fs::remove_file(workspace.join("compile_commands.json"));

    let sources = c_sources(c_dir);
    if sources.is_empty() {
        return;
    }
    let compiler = compiler
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "cc".to_string());

    let entries: Vec<String> = sources
        .iter()
        .map(|file| {
            let args = [
                compiler.clone(),
                "-std=c11".to_string(),
                format!("-I{}", c_dir.display()),
                format!("-I{}", coda_include.display()),
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
    let out = manifest.join("compile_commands.json");
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
