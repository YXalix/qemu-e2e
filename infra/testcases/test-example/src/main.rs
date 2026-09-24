//! test-example —— 合流示例用例：Rust 框架入口 + C 测试体（FFI）。
//!
//! 新用例约定：复制本目录，`TESTS` 注册 Rust 测试、`c/` 放 C 测试体
//! （`run_c_tests` 里逐个调用）；断言统一走 `testfw` 的宏（Rust 侧）或
//! `testfw.h` 的宏（C 侧），计数与打印同源。产物是 musl 静态 ELF，
//! 由 builder 拷入 rootfs `/tests/` 自动发现。

use testfw::{run_and_exit, TestCase};

// C 测试段入口（c/*.c，build.rs 经 cc crate 编入本二进制）。
extern "C" {
    fn run_c_tests();
}

fn basic_functionality() -> bool {
    testfw::check!(std::process::id() > 0, "guest process alive (pid > 0)");
    testfw::check!(
        core::mem::size_of::<usize>() >= 4,
        "usize is at least 32-bit"
    )
}

fn osrelease_readable() -> bool {
    // rootfs 阶段 init 已挂 /proc；免 fork 读内核版本（替代旧 main.c 的
    // system("uname -r")）
    match std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(s) => testfw::check!(!s.trim().is_empty(), "kernel release: {}", s.trim()),
        Err(e) => testfw::check!(false, "read /proc/sys/kernel/osrelease: {e}"),
    }
}

static TESTS: &[TestCase] = &[
    ("basic_functionality", basic_functionality),
    ("osrelease_readable", osrelease_readable),
];

fn main() {
    testfw::install_panic_hook();
    println!("=== Virtuoso E2E Tests ===");
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .trim()
        .to_string();
    if !kernel.is_empty() {
        println!("Kernel: {kernel}");
    }

    // 先跑 C 段（计数落入 testfw），再跑 Rust 段；退出码由总计数决定
    unsafe { run_c_tests() };
    let (passed, failed, skipped) = testfw::counters();
    testfw::info!("C section counters: passed={passed} failed={failed} skipped={skipped}");

    run_and_exit(TESTS);
}
