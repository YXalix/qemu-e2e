//! test-example —— 示例用例：Rust 框架入口 + C 测试体（FFI）。
//!
//! 新用例约定：复制本目录、改本 crate 的 `name`，`TESTS` 注册 Rust 测试、
//! `c/` 放 C 测试体（入口函数 `run_c_tests` 里逐个调用）；断言统一走
//! `coda` 的宏（Rust 侧）或 `coda.h` 的宏（C 侧），计数与打印同源。
//! 产物是 musl 静态 ELF，由 builder 拷入 rootfs `/tests/` 自动发现。

use coda::{run_and_exit, TestCase};

fn basic_functionality() -> bool {
    coda::check!(std::process::id() > 0, "guest process alive (pid > 0)");
    coda::check!(
        core::mem::size_of::<usize>() >= 4,
        "usize is at least 32-bit"
    )
}

fn osrelease_readable() -> bool {
    // rootfs 阶段 init 已挂 /proc；免 fork 读内核版本（替代旧 main.c 的
    // system("uname -r")）
    match std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(s) => coda::check!(!s.trim().is_empty(), "kernel release: {}", s.trim()),
        Err(e) => coda::check!(false, "read /proc/sys/kernel/osrelease: {e}"),
    }
}

static TESTS: &[TestCase] = &[
    ("basic_functionality", basic_functionality),
    ("osrelease_readable", osrelease_readable),
];

fn main() {
    run_and_exit(TESTS);
}
