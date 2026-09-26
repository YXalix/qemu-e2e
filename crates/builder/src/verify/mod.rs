//! 前置检查引擎（verify.sh 的 Rust 接管）。输出文本与退出码语义对齐 shell
//! 基线：critical FAIL → exit 1；WARN/INFO 不影响退出码。
//!
//! doctor 的两种呈现（一屏分组 / --verbose 全量）都消费本模块的 Report：
//! 每条 Check 自带 `kind`（分组）与 `summary`（一屏紧凑短语，None = 不进
//! 一屏）——新增前置条件只动引擎，呈现自动跟随，doctor 不做消息文本反解。
//!
//! 模块划分：`check`（Check/CheckKind/Level 类型与 Report 渲染）、
//! `checks`（逐项检查函数）；本文件承载引擎输入（CheckInput）与编排。

mod check;
mod checks;

pub use check::{
    detail, is_stdout_tty, Check, CheckKind, Level, Report,
};
pub use checks::{host_tools, kernel_docker_checks, module_presence};

use std::path::Path;

use common::{Arch, HostOs};

/// 检查引擎输入（doctor / engine_report 投影；一次装配，免长参数表）。
pub struct CheckInput<'a> {
    pub config_file_exists: bool,
    pub kernel_path: Option<&'a Path>,
    pub arch: Arch,
    pub host: HostOs,
    pub host_is_cross: bool,
    pub kernel_image: Option<&'a Path>,
    pub qemu_bin: Option<&'a str>,
    pub qemu_override: Option<&'a str>,
    /// (模块名, .ko 是否找到)
    pub modules: &'a [(String, bool)],
    pub busybox_cached: bool,
    pub tools_img_exists: bool,
    pub initrd: Option<&'a Path>,
    pub vfio_enabled: bool,
    pub pmem_enabled: bool,
}

/// 运行全部检查（verify.sh 的 11 项 + 组件平台门），逐项独立成函数。
pub fn run_checks(input: &CheckInput) -> Report {
    let mut checks = Vec::new();
    checks::check_config(input, &mut checks);
    checks::check_host_tools(input, &mut checks);
    checks::check_kernel_source(input, &mut checks);
    checks::check_kernel_image(input, &mut checks);
    checks::check_qemu(input, &mut checks);
    checks::check_modules(input, &mut checks);
    checks::check_busybox(input, &mut checks);
    checks::check_cross(input, &mut checks);
    checks::check_tools_image(input, &mut checks);
    checks::check_initrd(input, &mut checks);
    checks::check_components(input, &mut checks);

    let critical_pass = checks.iter().filter(|c| c.level == Level::Pass).count() as u32;
    let critical_fail = checks.iter().filter(|c| c.level == Level::Fail).count() as u32;
    let warnings = checks.iter().filter(|c| c.level == Level::Warn).count() as u32;
    Report {
        checks,
        critical_pass,
        critical_fail,
        warnings,
    }
}
