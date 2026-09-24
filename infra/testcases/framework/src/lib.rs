//! testfw — Virtuoso 测试框架（std + musl 静态链接）。
//!
//! 与 C 侧 `testfw.h` 的 `PASS/FAIL/SKIP/INFO` 宏语义一一对齐：标记协议
//! v1（冻结文本，设计文档附录 A）对 C/Rust 用例一视同仁——同一串口协议，
//! 同一套判定。Rust 入口（`run_and_exit`）与 C 测试体（经 FFI 导出的
//! `testfw_pass` 等函数）共享同一套计数器与打印路径，冻结文本单一来源。
//!
//! 静态链接约束：musl 目标 crt-static 是 rustc 默认行为（与 tools workspace
//! 同配方），产物为纯静态 ELF——VM 内无动态加载器，**不可放松**。
//! 输出走 std stdout（console 为 tty，按行 flush），退出走 `process::exit`。

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU32, Ordering};

/// 单个测试：返回 true = 通过。断言细节由宏直接打印到串口。
pub type TestCase = (&'static str, fn() -> bool);

/// 断言计数器（单线程进程，Atomic 仅为主 FFI 边界的静态生命周期服务）。
static PASSED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);
static SKIPPED: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------- 串口输出

/// 标记协议 v1 断言行：两空格缩进 + `[TAG] message`（与 C 宏输出一致，
/// judge 解析时 trim 前导空白）。
pub fn emit(tag: &str, args: std::fmt::Arguments<'_>) {
    println!("  [{tag}] {args}");
}

// ---------------------------------------------------------------- 协议宏

#[macro_export]
macro_rules! pass {
    ($($arg:tt)*) => { $crate::emit("PASS", format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => { $crate::emit("FAIL", format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! skip {
    ($($arg:tt)*) => { $crate::emit("SKIP", format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::emit("INFO", format_args!($($arg)*)) };
}

/// 断言：失败时打印 [FAIL] 并返回 false（不中断同一测试的后续断言，
/// 与 C 侧 PASS/FAIL 宏的计数语义一致）。
#[macro_export]
macro_rules! check {
    ($cond:expr, $($arg:tt)*) => {
        if $cond {
            $crate::pass!($($arg)*);
            true
        } else {
            $crate::fail!($($arg)*);
            false
        }
    };
}

// ---------------------------------------------------------------- FFI 导出（C 测试体的唯一入口）

/// C 侧消息指针 → 安全字符串（NULL 容错为空串）。
fn c_msg(msg: *const c_char) -> String {
    if msg.is_null() {
        return String::new();
    }
    // C 侧传来的应是合法 UTF-8（snprintf 产物）；坏字节替换而非 UB/panic。
    unsafe { CStr::from_ptr(msg) }
        .to_string_lossy()
        .trim_end()
        .to_string()
}

/// FFI 计数 + 打印的公共路径。计数在打印前递增，保证 judge 看到的
/// `[TAG]` 行与内部计数强一致。
macro_rules! ffi_emit {
    ($name:ident, $tag:literal, $counter:ident) => {
        #[doc = concat!("`", $tag, "` 断言行（C 侧 `", stringify!($name), "` 的落点）。")]
        #[no_mangle]
        pub extern "C" fn $name(msg: *const c_char) {
            $counter.fetch_add(1, Ordering::Relaxed);
            emit($tag, format_args!("{}", c_msg(msg)));
        }
    };
}

ffi_emit!(testfw_pass, "PASS", PASSED);
ffi_emit!(testfw_fail, "FAIL", FAILED);
ffi_emit!(testfw_skip, "SKIP", SKIPPED);

/// `INFO` 行（只打印不计数，与 C 宏语义一致）。
#[no_mangle]
pub extern "C" fn testfw_info(msg: *const c_char) {
    emit("INFO", format_args!("{}", c_msg(msg)));
}

/// 计数器只读快照（C 侧 `testfw_count_*` 的落点，`unsigned` = u32）。
#[no_mangle]
pub extern "C" fn testfw_count_passed() -> u32 {
    PASSED.load(Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn testfw_count_failed() -> u32 {
    FAILED.load(Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn testfw_count_skipped() -> u32 {
    SKIPPED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------- 运行器

/// 共享 main 等价物：逐个执行 Rust 侧测试后按计数据退出。
/// `Test Results` / `TEST_COMPLETE` 由 init 汇编（协议 v1 分工）；
/// C 测试体由用例 main 在调用本函数前先执行（见 test-example）。
pub fn run_and_exit(tests: &[TestCase]) -> ! {
    for (name, f) in tests {
        info!("Test: {name}");
        let before = FAILED.load(Ordering::Relaxed);
        let ok = f();
        if !ok && FAILED.load(Ordering::Relaxed) == before {
            // 函数返回 false 但没打 [FAIL]：补一条，防判定口径缺行
            fail!("{name}: assertion failed without [FAIL] line");
        }
    }
    exit_by_counters()
}

/// 依计数据退出码：任何 FAIL > 0 → 1，否则 0（init 依此打印 PASSED/FAILED）。
pub fn exit_by_counters() -> ! {
    let code = if FAILED.load(Ordering::Relaxed) > 0 { 1 } else { 0 };
    std::process::exit(code)
}

/// 汇总断言计数（供用例 main 打自由格式 summary 用）。
pub fn counters() -> (u32, u32, u32) {
    (
        PASSED.load(Ordering::Relaxed),
        FAILED.load(Ordering::Relaxed),
        SKIPPED.load(Ordering::Relaxed),
    )
}

/// panic 处理：打印 [FAIL] 后 abort（非零退出）。profile 为
/// `panic = "abort"`，hook 先于 abort 执行——panic 即用例失败，不假通过。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()));
        let loc = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        match payload {
            Some(s) => emit("FAIL", format_args!("panic: {s}{loc}")),
            None => emit("FAIL", format_args!("panic:{loc}")),
        }
    }));
}
