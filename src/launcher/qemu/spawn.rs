//! QEMU 进程拉起：spawn 独立进程组，收割交给 guardian（Supervised 登记）。

use anyhow::Context;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use crate::launcher::guardian::Supervised;

use super::QemuInvocation;

impl QemuInvocation {
    /// spawn QEMU：独立进程组（pgid = 返回的 child pid，交给 guardian 模块收割）。
    /// `piped` = true 时 stdout/stderr 管道化（test 路径捕获串口），
    /// false 时继承宿主 stdio（交互 shell / debug）。
    pub(crate) fn spawn(self, piped: bool) -> anyhow::Result<(Child, u32)> {
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
        let child = cmd.spawn().with_context(|| {
            format!(
                "{} not found; run `virtuoso doctor --verbose` for install hints",
                self.qemu_bin()
            )
        })?;
        let pgid = child.id();
        Ok((child, pgid))
    }

    /// spawn 并登记监管：注册表 + 收割守卫一步到位（取代调用方四步样板）。
    pub(crate) fn spawn_supervised(self, piped: bool) -> anyhow::Result<(Child, Supervised)> {
        let (child, pgid) = self.spawn(piped)?;
        Ok((child, Supervised::adopt(pgid)))
    }
}
