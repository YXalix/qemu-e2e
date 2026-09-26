//! guardian — 守护者：进程组与终端前台的 RAII 治理。
//!
//! `ProcessGroupGuard` 取代早期 Makefile 驱动方案的 PID 文件 + `kill -- -PGID` hack：
//! guard 存活即持有进程组，`Drop`（含 panic 展开、错误提前返回、Ctrl-C 退出路径）
//! 保证收割，宿主机不残留 QEMU 子进程。`Supervised` 在此之上叠加活动进程组
//! 注册表（Ctrl-C 守护收割）；注册表与墙钟看门狗同文件承载（registry 节）。
//! `TerminalHandover` 补齐终端前台这一面：交互会话把控制终端前台移交给
//! QEMU 进程组（移交动作在子进程 exec 前完成，见 qemu::spawn），
//! finish/Drop 时还原给调用方进程组。

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 对进程组发送信号。sig 支持 "KILL" / "TERM"，"0" 为存活探测（不发信号，
/// 仅做存在性与权限检查）。pgid=0 一律拒绝：killpg(0) 会命中调用者自身
/// 所在进程组。公开给看门狗线程等无法持有 guard 的调用方使用。
/// 返回 false = 组不存在、无权限或信号名不支持。
pub(crate) fn signal_group(pgid: u32, sig: &str) -> bool {
    if pgid == 0 {
        return false;
    }
    let signum = match sig {
        "KILL" => libc::SIGKILL,
        "TERM" => libc::SIGTERM,
        "0" => 0,
        _ => return false,
    };
    // SAFETY: killpg 仅向目标进程组发送 signum；pgid=0 已在上方拒绝。
    unsafe { libc::killpg(pgid as libc::pid_t, signum) == 0 }
}

/// 进程组收割守卫。`process_group(0)` spawn 的子进程 pid 即 pgid；
/// 未 `disarm` 即 `Drop` 时 KILL 整个进程组。
pub(crate) struct ProcessGroupGuard {
    pgid: u32,
    armed: bool,
}

impl ProcessGroupGuard {
    /// 接管一个进程组。
    pub(crate) fn adopt(pgid: u32) -> Self {
        Self {
            pgid,
            armed: pgid != 0,
        }
    }

    /// KILL 整个进程组（幂等；收割后 guard 自动失效）。
    pub(crate) fn kill_now(&mut self) {
        if self.armed {
            signal_group(self.pgid, "KILL");
            self.armed = false;
        }
    }

    /// 解除收割责任（确信进程组已自然退出时调用）。
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            signal_group(self.pgid, "KILL");
        }
    }
}

/// 终端前台移交守卫：记录调用方当前的前台进程组，会话结束时还原。
/// 还原覆盖 job-control shell 自行回收之外的场景（无 job control 的
/// 祖先脚本不会回收——前台组悬死会让后续读 tty 的进程冻结）。
pub(crate) struct TerminalHandover {
    saved_pgid: libc::pid_t,
}

impl TerminalHandover {
    /// 记录当前终端前台组。fd0 非控制终端 → Ok(None)（piped 路径 / 无 tty
    /// 环境，QEMU 读管道或 /dev/null，不存在 SIGTTIN 问题，无需移交）。
    pub(crate) fn capture() -> anyhow::Result<Option<Self>> {
        // SAFETY: isatty 只查询 fd 类型，无副作用。
        if unsafe { libc::isatty(0) } != 1 {
            return Ok(None);
        }
        // SAFETY: tcgetpgrp 只读终端前台归属。
        let saved_pgid = unsafe { libc::tcgetpgrp(0) };
        if saved_pgid < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOTTY) {
                // fd0 是 tty 但非本会话控制终端（isatty 已过）：读写这种
                // 终端不触发 SIGTTIN/SIGTTOU，无需移交，按无终端处理。
                return Ok(None);
            }
            anyhow::bail!("tcgetpgrp(0) failed: {err}");
        }
        Ok(Some(Self { saved_pgid }))
    }
}

impl Drop for TerminalHandover {
    fn drop(&mut self) {
        // QEMU（前台组）退出后调用方回到"后台"；后台组调 tcsetpgrp 的内核
        // 响应是 SIGTTOU（默认动作 stop），还原期间必须先阻塞。
        let mut blocked: libc::sigset_t = unsafe { std::mem::zeroed() };
        let mut saved_mask: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: sigset_t 清零 + sigemptyset 后使用；掩码改动仅本线程。
        unsafe {
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTTOU);
            if libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut saved_mask) != 0 {
                return; // 拿不到掩码就不动前台：宁可悬挂前台也不冒险 stop 自己
            }
            libc::tcsetpgrp(0, self.saved_pgid);
            libc::pthread_sigmask(libc::SIG_SETMASK, &saved_mask, std::ptr::null_mut());
        }
    }
}

/// 注册表登记 + 收割守卫的组合句柄：spawn 后 `adopt` 一次、收尾 `finish`
/// 一次，取代调用方的 register / adopt / 注销 / disarm 四步样板。
pub(crate) struct Supervised {
    pgid: u32,
    guard: ProcessGroupGuard,
    tty: Option<TerminalHandover>,
}

impl Supervised {
    /// 接管一个进程组并登记为活动进程组（Ctrl-C 守护可见）。
    pub(crate) fn adopt(pgid: u32) -> Self {
        register(pgid);
        Self {
            pgid,
            guard: ProcessGroupGuard::adopt(pgid),
            tty: None,
        }
    }

    /// 挂上终端前台还原责任（交互会话专用）。guard 声明在 tty 之前：
    /// 异常 Drop 路径先收割 QEMU 再还原前台。
    pub(crate) fn with_tty(mut self, tty: Option<TerminalHandover>) -> Self {
        self.tty = tty;
        self
    }

    pub(crate) fn pgid(&self) -> u32 {
        self.pgid
    }

    /// KILL 整个进程组（幂等；收割后自动失效）。
    pub(crate) fn kill_now(&mut self) {
        self.guard.kill_now();
    }

    /// 注销注册表并解除收割责任（进程组已自然退出时调用）；
    /// 终端前台同点还原给调用方进程组。
    pub(crate) fn finish(&mut self) {
        clear();
        self.guard.disarm();
        self.tty = None; // Drop 即还原前台
    }
}

// ---------------------------------------------------------------- 注册表与看门狗

// 活动进程组注册表：Ctrl-C 守护与墙钟看门狗共用的全局状态。
//
// 同一时刻至多一个被包装的 VM 进程组；`Supervised::adopt` 登记、
// `finish` 注销，Ctrl-C 守护据此收割，取代调用方散落的静态变量样板。

static ACTIVE_PGID: AtomicU32 = AtomicU32::new(0);

/// 登记当前被包装的进程组（Ctrl-C 守护据此收割）。
pub(crate) fn register(pgid: u32) {
    ACTIVE_PGID.store(pgid, Ordering::SeqCst);
}

/// 注销（进程组自然退出或已收割后调用）。
pub(crate) fn clear() {
    ACTIVE_PGID.store(0, Ordering::SeqCst);
}

/// 当前活动进程组（0 = 无；仅供本模块守护/看门狗收割用）。
fn active() -> u32 {
    ACTIVE_PGID.load(Ordering::SeqCst)
}

/// 安装 Ctrl-C 守护：KILL 活动进程组后以 130 退出（与 shell 中断语义一致；
/// 常量钉在 crate::util，judge 为唯一退出码语义表）。
pub(crate) fn install_ctrlc_guard() {
    let _ = ctrlc::set_handler(|| {
        let pgid = active();
        if pgid != 0 {
            signal_group(pgid, "KILL");
        }
        std::process::exit(crate::util::EXIT_INTERRUPTED);
    });
}

/// 墙钟看门狗：到点 KILL 进程组并置位 quit（等价 timeout --signal=KILL 的
/// 124 语义；调用方据 quit 位区分超时与自然退出）。
pub(crate) fn spawn_watchdog(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn spawn_sleep_group(secs: &str) -> (std::process::Child, u32) {
        let child = Command::new("sleep")
            .arg(secs)
            .process_group(0)
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let pgid = child.id();
        (child, pgid)
    }

    fn group_alive(pgid: u32) -> bool {
        signal_group(pgid, "0")
    }

    #[test]
    fn drop_kills_process_group() {
        let (mut child, pgid) = spawn_sleep_group("60");
        {
            let _guard = ProcessGroupGuard::adopt(pgid);
            assert!(group_alive(pgid));
        }
        let _ = child.wait();
        assert!(
            !group_alive(pgid),
            "process group must not survive guard drop"
        );
    }

    #[test]
    fn kill_now_is_idempotent_and_disarms() {
        let (mut child, pgid) = spawn_sleep_group("60");
        let mut guard = ProcessGroupGuard::adopt(pgid);
        guard.kill_now();
        guard.kill_now(); // 第二次不 panic、不发信号
        let _ = child.wait();
        assert!(!group_alive(pgid));
    }

    #[test]
    fn adopt_zero_is_inert() {
        // pgid=0 必须是惰性的：signal_group 拒绝 -0（否则会杀死当前进程组）
        let mut guard = ProcessGroupGuard::adopt(0);
        guard.kill_now();
        guard.disarm();
        assert!(
            !group_alive(0),
            "guard must never signal the caller's own group"
        );
    }
}
