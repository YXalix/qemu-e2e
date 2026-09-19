//! Virtuoso Phase 1 — qemu-e2e 的 cargo-xtask 对等包装层。
//!
//! 设计文档：docs/virtuoso-design.md（§2.4 CLI 映射、§6 Phase 1）

mod config;
mod tasks;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "virtuoso",
    version,
    about = "Virtuoso — kernel E2E virtualization test harness (qemu-e2e Phase-1 wrapper)",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 前置检查：先输出类型化配置诊断，再运行 infra/verify.sh（make verify 对等）
    Verify {
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
    },
    /// 构建 initrd：C 用例 + modules.conf + BusyBox（make initrd 对等）
    Build,
    /// 交互式启动 QEMU，落入 BusyBox shell（make qemu 对等）
    Shell {
        /// KVM 加速（仅宿主与目标同构时可用）
        #[arg(long)]
        kvm: bool,
    },
    /// 调试启动：挂起等待 GDB 连接 :1234（make qemu-debug 对等）
    Debug,
    /// CI 模式：重建 initrd → 超时运行 → 标记协议判定退出码（make qemu-test 对等）
    Test {
        /// 墙钟超时秒数；0 一律拒绝。缺省读 .env 的 QEMU_TIMEOUT
        #[arg(long)]
        timeout: Option<u64>,
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
    },
    /// 创建 512M disk.qcow2（NVMe 用块设备，幂等；make disk 对等）
    Disk,
    /// 确保 ARCH 对应的静态 BusyBox：release 下载优先，源码兜底（make busybox 对等）
    #[command(name = "busybox")]
    BusyBox,
    /// 清理生成物（make clean 对等）
    Clean,
    /// AI skill 管理（make install-skill / uninstall-skill 对等）
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// 多架构矩阵批量测试（Phase 2：ensemble）
    Matrix {
        /// 目标架构；缺省为三架构全矩阵
        #[arg(long)]
        arch: Option<String>,
    },
    /// AI 串口日志分诊（Phase 2：auditor events.jsonl）
    Triage,
    /// 历史串口日志回放断言（Phase 2：encore）
    Replay {
        /// 串口日志文件
        #[arg(long = "log")]
        log: PathBuf,
    },
    /// parity 校验：同一 target 分别以 make 与 cargo xtask 执行并比较退出码
    Parity {
        /// make target 名（verify/busybox/initrd/qemu/qemu-kvm/qemu-debug/qemu-test/disk/clean/install-skill/uninstall-skill）
        target: String,
        /// 允许执行会启动 VM 或触发 BusyBox 全量构建的 target
        #[arg(long)]
        force: bool,
        /// 要求退出码严格相等（缺省容忍 make 将脚本失败折叠为 2 的行为）
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// 安装 kernel-dev skill 到内核树
    Install,
    /// 从内核树移除 kernel-dev skill
    Uninstall,
}

fn main() {
    let cli = Cli::parse();
    tasks::install_ctrlc_guard();
    match tasks::dispatch(cli.command) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            std::process::exit(1);
        }
    }
}
