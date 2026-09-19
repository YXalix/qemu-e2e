//! testfw — Virtuoso no_std 测试框架。
//!
//! 与 C 侧 `test_common.h` 的 `PASS/FAIL/SKIP/INFO` 宏语义一一对齐：标记协议
//! v1（冻结文本，设计文档附录 A）对 C/Rust 用例一视同仁——同一串口协议，
//! 同一套判定。运行器语义对齐 C 共享 `main.c`：逐个执行测试、断言计数、
//! 失败即退出码 1（init 依退出码打印 PASSED:/FAILED:，并汇编
//! `Test Results` / `TEST_COMPLETE`）。
//!
//! 裸机约束：`#![no_std]` + 自定义 `_start`，libc 不参与；输出走 write(1)
//! 裸 syscall（init 将 console 作为 stdout），退出走 exit_group 裸 syscall。

#![no_std]

use core::arch::asm;
use core::fmt::{Arguments, Write};

/// 单个测试：返回 true = 通过。断言细节由宏直接打印到串口。
pub type TestCase = (&'static str, fn() -> bool);

// ---------------------------------------------------------------- 裸 syscall

#[inline]
unsafe fn syscall1(n: u64, a0: u64) -> i64 {
    let out: i64;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        asm!(
            "svc #0",
            inlateout("x8") n => _,
            inlateout("x0") a0 => out,
            lateout("x1") _, lateout("x2") _,
            lateout("x3") _, lateout("x4") _,
            lateout("x5") _, lateout("x6") _,
            lateout("x7") _,
            options(nostack)
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => out,
            in("rdi") a0,
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
    #[cfg(target_arch = "riscv64")]
    unsafe {
        asm!(
            "ecall",
            inlateout("a7") n => _,
            inlateout("a0") a0 => out,
            lateout("a1") _, lateout("a2") _,
            lateout("a3") _, lateout("a4") _,
            lateout("a5") _, lateout("a6") _,
            options(nostack)
        );
    }
    #[cfg(not(any(
        target_arch = "aarch64",
        target_arch = "x86_64",
        target_arch = "riscv64"
    )))]
    compile_error!("testfw: unsupported architecture (expect aarch64/x86_64/riscv64)");
    out
}

#[inline]
unsafe fn syscall3(n: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let out: i64;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        asm!(
            "svc #0",
            inlateout("x8") n => _,
            inlateout("x0") a0 => out,
            in("x1") a1, in("x2") a2,
            lateout("x3") _, lateout("x4") _,
            lateout("x5") _, lateout("x6") _,
            lateout("x7") _,
            options(nostack)
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => out,
            in("rdi") a0, in("rsi") a1, in("rdx") a2,
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
    #[cfg(target_arch = "riscv64")]
    unsafe {
        asm!(
            "ecall",
            inlateout("a7") n => _,
            inlateout("a0") a0 => out,
            in("a1") a1, in("a2") a2,
            lateout("a3") _, lateout("a4") _,
            lateout("a5") _, lateout("a6") _,
            options(nostack)
        );
    }
    #[cfg(not(any(
        target_arch = "aarch64",
        target_arch = "x86_64",
        target_arch = "riscv64"
    )))]
    compile_error!("testfw: unsupported architecture (expect aarch64/x86_64/riscv64)");
    out
}

/// write(fd=1) 裸 syscall（64/1/64）。
fn write_stdout(buf: &[u8]) {
    unsafe {
        let _ = syscall3(
            match () {
                #[cfg(target_arch = "x86_64")]
                () => 1,
                #[cfg(not(target_arch = "x86_64"))]
                () => 64,
            },
            1,
            buf.as_ptr() as u64,
            buf.len() as u64,
        );
    }
}

/// exit_group 裸 syscall（aarch64/riscv64: 94，x86_64: 231）——单线程进程里
/// 与 exit 等价，但语义上确保整个测试进程终止。
pub fn exit(code: i32) -> ! {
    unsafe {
        let n = match () {
            #[cfg(target_arch = "x86_64")]
            () => 231u64,
            #[cfg(not(target_arch = "x86_64"))]
            () => 94u64,
        };
        let _ = syscall1(n, code as u64);
    }
    loop {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------- freestanding mem 函数

// linux-gnu 目标的 compiler_builtins 不提供 mem*（默认留给 libc）；无 libc 时
// 由本框架补齐。逐字节赋值不会被优化器折叠成对 memcpy 自身的调用。
#[no_mangle]
unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = *src.add(i);
        i += 1;
    }
    dest
}

#[no_mangle]
unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < src as usize {
        let mut i = 0;
        while i < n {
            *dest.add(i) = *src.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
    }
    dest
}

#[no_mangle]
unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = c as u8;
        i += 1;
    }
    dest
}

#[no_mangle]
unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

// ---------------------------------------------------------------- 串口输出

/// 标记协议 v1 断言行：两空格缩进 + `[TAG] message`（与 C 宏输出一致，
/// auditor 解析时 trim 前导空白）。
pub fn emit(tag: &str, args: Arguments<'_>) {
    let mut buf: [u8; 512] = [0; 512];
    let pos = {
        let mut w = BufWriter {
            buf: &mut buf,
            pos: 0,
        };
        let _ = write!(w, "  [{tag}] {args}\n");
        w.pos
    };
    write_stdout(&buf[..pos]);
}

struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for BufWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let n = bytes.len().min(self.buf.len() - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        Ok(())
    }
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

// ---------------------------------------------------------------- 运行器

/// 共享 main 等价物：逐个执行测试，exit_group(失败数 > 0 ? 1 : 0)。
/// `Test Results` / `TEST_COMPLETE` 由 init 汇编（协议 v1 分工）。
pub fn run_and_exit(tests: &[TestCase]) -> ! {
    let mut failed: u32 = 0;
    for (name, f) in tests {
        info!("Test: {name}");
        let ok = f();
        if !ok {
            failed += 1;
        }
    }
    exit(if failed > 0 { 1 } else { 0 })
}

/// panic 处理：打印 [FAIL] 后以 101 退出（panic 即用例失败，不假通过）。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    #[allow(deprecated)] // panic!("字面量") 时 payload downcast 仍返回 &str
    let payload = info.payload().downcast_ref::<&str>().copied();
    match payload {
        Some(s) => emit("FAIL", format_args!("panic: {s}")),
        None => emit("FAIL", format_args!("panic: {info}")),
    }
    exit(101)
}
