//! no_std Rust 用例骨架 —— 与 C 侧 `test_example.c` 语义对齐的最小样例。
//!
//! 新 Rust 用例的约定：复制本 crate，在 `TESTS` 里注册 `("名称", 函数)`；
//! 断言用 `testfw::check!`（或 `pass!/fail!/skip!` 手动组合）。
//! 产物是静态 ELF，由 overture 拷入 rootfs `/tests/` 自动发现。

#![no_std]
#![no_main]

use testfw::{run_and_exit, TestCase};

fn basic_functionality() -> bool {
    testfw::check!(1 + 1 == 2, "Basic arithmetic works");
    testfw::check!(
        core::mem::size_of::<usize>() >= 4,
        "usize is at least 32-bit"
    )
}

fn no_panic_under_alloc_free_env() -> bool {
    // 无堆环境冒烟：栈上构造数据并做跨函数传递（覆盖链接器组合正确性）
    let data = [1u8, 2, 3, 4];
    let sum: u32 = data.iter().map(|&b| b as u32).sum();
    testfw::check!(sum == 10, "stack slice fold sums to 10")
}

static TESTS: &[TestCase] = &[
    ("basic_functionality", basic_functionality),
    (
        "no_panic_under_alloc_free_env",
        no_panic_under_alloc_free_env,
    ),
];

#[no_mangle]
extern "C" fn _start() -> ! {
    run_and_exit(TESTS)
}
