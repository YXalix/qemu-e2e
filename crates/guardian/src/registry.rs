//! 活动进程组注册表：Ctrl-C 守护与墙钟看门狗共用的全局状态。
//!
//! 同一时刻至多一个被包装的 VM 进程组；`Supervised::adopt` 登记、
//! `finish` 注销，Ctrl-C 守护据此收割，取代调用方散落的静态变量样板。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::signal_group;

static ACTIVE_PGID: AtomicU32 = AtomicU32::new(0);

/// 登记当前被包装的进程组（Ctrl-C 守护据此收割）。
pub fn register(pgid: u32) {
    ACTIVE_PGID.store(pgid, Ordering::SeqCst);
}

/// 注销（进程组自然退出或已收割后调用）。
pub fn clear() {
    ACTIVE_PGID.store(0, Ordering::SeqCst);
}

/// 当前活动进程组（0 = 无）。
pub fn active() -> u32 {
    ACTIVE_PGID.load(Ordering::SeqCst)
}

/// 安装 Ctrl-C 守护：KILL 活动进程组后以 130 退出（与 shell 中断语义一致；
/// 常量钉在 common::exit，judge::exit 为唯一语义表）。
pub fn install_ctrlc_guard() {
    let _ = ctrlc::set_handler(|| {
        let pgid = active();
        if pgid != 0 {
            signal_group(pgid, "KILL");
        }
        std::process::exit(common::exit::EXIT_INTERRUPTED);
    });
}

/// 墙钟看门狗：到点 KILL 进程组并置位 quit（等价 timeout --signal=KILL 的
/// 124 语义；调用方据 quit 位区分超时与自然退出）。
pub fn spawn_watchdog(
    pgid: u32,
    timeout_secs: u64,
    quit: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if quit.load(Ordering::SeqCst) {
                return;
            }
            if Instant::now() >= deadline {
                quit.store(true, Ordering::SeqCst);
                signal_group(pgid, "KILL");
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}
