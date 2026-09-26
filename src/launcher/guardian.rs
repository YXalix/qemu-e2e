//! guardian — 守护者：进程组 RAII 治理。
//!
//! `ProcessGroupGuard` 取代早期 Makefile 驱动方案的 PID 文件 + `kill -- -PGID` hack：
//! guard 存活即持有进程组，`Drop`（含 panic 展开、错误提前返回、Ctrl-C 退出路径）
//! 保证收割，宿主机不残留 QEMU 子进程。`Supervised` 在此之上叠加活动进程组
//! 注册表（Ctrl-C 守护收割），`registry` 承载注册表与看门狗。

pub(crate) mod registry;

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

/// 注册表登记 + 收割守卫的组合句柄：spawn 后 `adopt` 一次、收尾 `finish`
/// 一次，取代调用方的 register / adopt / 注销 / disarm 四步样板。
pub(crate) struct Supervised {
    pgid: u32,
    guard: ProcessGroupGuard,
}

impl Supervised {
    /// 接管一个进程组并登记为活动进程组（Ctrl-C 守护可见）。
    pub(crate) fn adopt(pgid: u32) -> Self {
        registry::register(pgid);
        Self {
            pgid,
            guard: ProcessGroupGuard::adopt(pgid),
        }
    }

    pub(crate) fn pgid(&self) -> u32 {
        self.pgid
    }

    /// KILL 整个进程组（幂等；收割后自动失效）。
    pub(crate) fn kill_now(&mut self) {
        self.guard.kill_now();
    }

    /// 注销注册表并解除收割责任（进程组已自然退出时调用）。
    pub(crate) fn finish(&mut self) {
        registry::clear();
        self.guard.disarm();
    }
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
