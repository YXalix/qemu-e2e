//! Virtuoso — kernel E2E 虚拟化测试装置的 CLI 入口。
//!
//! 本 crate 只保留 clap 定义与子命令分发：命令实现在 `cli`（verify/build/vm），
//! 运行工件与呈现命令在 `runs`（rundir/render），类型化配置在
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
    /// 一屏环境体检（✓/✗/! 组件行；--verbose 全量清单 + 类型化配置诊断）
    Doctor {
        /// 覆盖目标架构（透传为 ARCH 环境变量）
        #[arg(long)]
        arch: Option<String>,
        /// 全量输出：类型化配置诊断 + 完整检查清单
        #[arg(long)]
        verbose: bool,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 构建 initrd：C 用例 + modules.conf + BusyBox
    Build {
        /// 只确保当前架构的静态 BusyBox（release 下载优先，源码兜底）
        #[arg(long = "busybox-only")]
        busybox_only: bool,
    },
    /// 拉取 preset 预编内核（mainline mini Image，全 =y 零模块）到
    /// target/kernel/preset —— kernel_preset = "mainline" 的开箱供给
    Fetch {
        /// 覆盖版本（X.Y[.Z]）；缺省 = infra/kernel/pin 钉定，再缺省 = 最新已发布
        #[arg(long)]
        version: Option<String>,
        /// 只拉指定架构（缺省三架构全量）
        #[arg(long)]
        arch: Option<String>,
    },
    /// 交互式启动 QEMU，落入 BusyBox shell（--gdb 挂起等 GDB 连接 :1234）
    Shell {
        /// KVM 加速（Linux；仅宿主与目标同构时可用）
        #[arg(long)]
        kvm: bool,
        /// 强制 TCG 纯模拟（macOS 缺省 HVF 时使用）
        #[arg(long)]
        tcg: bool,
        /// 挂起等待 GDB 连接 :1234（恒 TCG 纯模拟）
        #[arg(long)]
        gdb: bool,
    },
    /// CI 模式：重建 initrd → 超时运行 → 标记协议判定退出码。
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
    /// 清理生成物
    Clean,
    /// AI skill 管理（装入 / 移出内核树）
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
