//! QEMU 进程拉起：spawn 独立进程组，收割交给 guardian（Supervised 登记）。
//! 交互路径（stdio 继承）额外在 exec 前把控制终端前台移交给 QEMU 进程组，
//! 会话结束由 TerminalHandover 还原——独立进程组相对终端是"后台组"，
//! 触碰控制终端（tcsetattr 进 raw mode / 读 stdin）会被 SIGTTIN/SIGTTOU
//! 冻结，不移交就是串口零输出 + Ctrl+C 假退出。

use anyhow::Context;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use crate::launcher::guardian::{Supervised, TerminalHandover};

use super::QemuInvocation;

impl QemuInvocation {
    /// spawn QEMU：独立进程组（pgid = 返回的 child pid，交给 guardian 模块收割）。
    /// `piped` = true 时 stdout/stderr 管道化（test 路径捕获串口），
    /// false 时继承宿主 stdio（交互 shell / debug），并返回终端前台移交守卫。
    pub(crate) fn spawn(
        self,
        piped: bool,
    ) -> anyhow::Result<(Child, u32, Option<TerminalHandover>)> {
        for (what, path) in [
            ("Kernel image", &self.kernel),
            ("Initramfs", &self.initrd),
            ("Rootfs image", &self.rootfs),
        ]
        .into_iter()
        .chain(self.data_disks.iter().map(|d| ("Data disk image", &d.path)))
        {
            if !path.is_file() {
                anyhow::bail!(
                    "{what} not found at {} (build the kernel / run `virtuoso build` first)",
                    path.display()
                );
            }
        }
        let args = self.argv()?;
        let mut cmd = Command::new(self.qemu_bin());
        cmd.args(&args).process_group(0);
        if piped {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null());
        }
        let tty = if !piped { TerminalHandover::capture()? } else { None };
        if tty.is_some() {
            // SAFETY: 闭包只做 async-signal-safe 系统调用（fork→exec 窗口的合法集）
            unsafe { cmd.pre_exec(hand_over_terminal_foreground) };
        }
        let child = cmd.spawn().with_context(|| {
            format!(
                "{} not found; run `virtuoso doctor --verbose` for install hints",
                self.qemu_bin()
            )
        })?;
        let pgid = child.id();
        Ok((child, pgid, tty))
    }

    /// spawn 并登记监管：注册表 + 收割守卫 + 终端前台还原一步到位
    /// （取代调用方四步样板）。
    pub(crate) fn spawn_supervised(self, piped: bool) -> anyhow::Result<(Child, Supervised)> {
        let (child, pgid, tty) = self.spawn(piped)?;
        Ok((child, Supervised::adopt(pgid).with_tty(tty)))
    }
}

/// exec 前的终端前台移交：自立进程组，把控制终端前台指向本进程组。
/// 只含 async-signal-safe 系统调用——fork 与 exec 之间唯一合法的调用集。
/// 必须在 exec 前完成：拖到 spawn 后由父进程补做存在竞态，QEMU 先碰到
/// 终端就被冻结。期间阻塞 SIGTTOU：此刻本组相对终端还是"后台"，
/// 后台组调 tcsetpgrp 的内核响应即 SIGTTOU（默认动作 stop）；掩码在
/// exec 前还原，QEMU 拿到干净起点。
fn hand_over_terminal_foreground() -> io::Result<()> {
    set_terminal_foreground(0)
}

/// `hand_over_terminal_foreground` 的 fd 参数化内核：生产路径 fd 恒为 0
/// （stdio 继承），预留参数化便于将来 pty 场景复用。
fn set_terminal_foreground(tty_fd: libc::c_int) -> io::Result<()> {
    // SAFETY: setpgid(0,0) 自立进程组；与 std 的 process_group(0) 幂等，
    // 且不依赖二者先后序（tcsetpgrp 的目标组必须先成立）。
    if unsafe { libc::setpgid(0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut blocked: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut saved: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: sigset_t 清零 + sigemptyset 后使用；掩码改动仅本线程（fork 后单线程）。
    unsafe {
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
        if libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut saved) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    // SAFETY: tcsetpgrp 只改该终端的前台进程组归属。
    let rc = unsafe { libc::tcsetpgrp(tty_fd, libc::getpgrp()) };
    // SAFETY: 还原进入时掩码，exec 后 QEMU 以干净掩码起步。
    unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &saved, std::ptr::null_mut()) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOTTY) {
            // fd 是 tty 但非本会话控制终端：POSIX 下读写这种终端不触发
            // SIGTTIN/SIGTTOU，无需移交，按无终端处理。
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}
