//! Virtuoso — kernel E2E 虚拟化测试装置的 CLI 入口。
//!
//! 本 crate 只保留 clap 定义与子命令分发：命令实现在 `cli`（verify/build/vm/
//! parity），运行工件与呈现命令在 `runs`（rundir/render），类型化配置在
//! `config`。领域逻辑全部在库 crate（common/builder/launcher/judge/guardian/
//! tracker）。
//!
//! 设计文档：docs/architecture/overview.md

mod cli;
mod config;
mod runs;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "virtuoso",
    version,
    about = "Virtuoso — kernel E2E virtualization test harness",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 前置检查：类型化配置诊断 + 构建环境检查（make verify 对等）
    Verify {
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
    },
    /// 一屏环境体检：verify 的 flutter-doctor 风格简化呈现（✓/✗/! 组件行）
    Doctor {
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 构建 initrd：C 用例 + modules.conf + BusyBox（make initrd 对等）
    Build,
    /// 交互式启动 QEMU，落入 BusyBox shell（make qemu 对等）
    Shell {
        /// KVM 加速（Linux；仅宿主与目标同构时可用）
        #[arg(long)]
        kvm: bool,
        /// 强制 TCG 纯模拟（macOS 缺省 HVF 时使用）
        #[arg(long)]
        tcg: bool,
    },
    /// 调试启动：挂起等待 GDB 连接 :1234（make qemu-debug 对等）
    Debug,
    /// CI 模式：重建 initrd → 超时运行 → 标记协议判定退出码（make qemu-test 对等）。
    /// --replay-until-fail N：对可疑 flaky 场景自动返场最多 N 次，出现首个
    /// 非 passed verdict 即停（tracker 返场语义）。
    Test {
        /// 墙钟超时秒数；0 一律拒绝。缺省读 virtuoso.toml 的 timeout_secs
        #[arg(long)]
        timeout: Option<u64>,
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
        /// 返场次数上限（缺省 1 = 单次执行）
        #[arg(long = "replay-until-fail")]
        replay_until_fail: Option<u32>,
        /// 强制 TCG 纯模拟（macOS 缺省 HVF 时使用；Linux 本就缺省 TCG）
        #[arg(long)]
        tcg: bool,
    },
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
    /// AI probe：经 virtio-serial agent 下发命令批，结构化事件流回吐
    /// （tools/virtuoso-agent 通道；AI 交互入口，退出码语义 0/1/124）
    Probe {
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
        /// 墙钟总超时秒数（含 TCG 引导与握手；0 一律拒绝）
        #[arg(long)]
        timeout: Option<u64>,
        /// 要执行的 shell 命令（可重复）
        #[arg(long = "cmd")]
        cmds: Vec<String>,
        /// 命令清单文件（每行一条，# 注释）
        #[arg(long = "cmd-file")]
        cmd_file: Option<PathBuf>,
        /// 机器可读 JSON 输出（事件行原样透传，供 AI 管道消费）
        #[arg(long)]
        json: bool,
    },
    /// 多架构矩阵批量测试（launcher）
    Matrix {
        /// 目标架构；缺省为三架构全矩阵
        #[arg(long)]
        arch: Option<String>,
    },
    /// 最近一次测试运行的分诊报告（--json 输出 verdict 供管道消费）
    Triage {
        /// 指定运行（target/runs 下的目录名）；缺省 = latest
        #[arg(long)]
        run: Option<String>,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 列出历史测试运行（最新在前；--json 输出数组）
    Runs {
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 对任一串口日志做标记协议回放断言（不启动 QEMU）
    Replay {
        /// 串口日志文件
        #[arg(long = "log")]
        log: PathBuf,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// parity 校验：同一 target 分别以 make 与 virtuoso 执行并比较退出码
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
    /// 跨 run 失败指纹聚类（tracker）：flaky 用例清单 + 失败首现 run
    Cluster {
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 补丁↔测试映射（tracker）：git diff 的子系统路径 → 推荐最小测试集
    Suggest {
        /// 统一 diff 文件；缺省对 KERNEL_PATH 内核树做 git diff（含暂存区）
        #[arg(long = "diff")]
        diff: Option<PathBuf>,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 文档：mdBook 构建到 target/book（docs/ 是唯一事实来源；发布走 gh-pages）
    Docs {
        /// 本地预览（mdbook serve，http://localhost:3000，改文件实时刷新）
        #[arg(long)]
        serve: bool,
        /// 构建完成后打开浏览器
        #[arg(long)]
        open: bool,
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
    guardian::registry::install_ctrlc_guard();
    match cli::dispatch(cli.command) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            std::process::exit(1);
        }
    }
}
