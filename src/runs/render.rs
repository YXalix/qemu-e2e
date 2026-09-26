//! 分诊命令的终端呈现：triage。
//! 解析与判定语义在 judge，本模块只做扫描、IO 与打印。

use common::fmt::human_size_ls;
use common::time::format_utc;
use judge::Verdict;

use super::rundir::{load_verdict_or_parse, resolve_run};
use crate::config::Config;

/// `virtuoso triage`：一次运行的分诊报告。
/// --json 输出 verdict 全文；verdict 非 passed 时退出 1（便于脚本串联）。
pub fn run_triage(cfg: &Config, run_spec: Option<&str>, json: bool) -> anyhow::Result<i32> {
    let dir = resolve_run(&cfg.project_root, run_spec)?;
    let v = load_verdict_or_parse(&dir)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(if v.verdict == Verdict::Passed { 0 } else { 1 });
    }

    let verdict = v.verdict.as_str();
    let dur_suffix = format!(", {:.1}s)", v.duration_ms as f64 / 1000.0);
    println!(
        "[TRIAGE] run {} — verdict: {}{}{}",
        v.run_id,
        verdict.to_uppercase(),
        v.exit_code
            .map(|c| format!(" (exit {c}"))
            .unwrap_or_default(),
        dur_suffix,
    );
    if let Some(note) = &v.verdict_source {
        println!("  note: {note}");
    }
    {
        print!("  arch: {}", v.arch);
        if let Some(acc) = v.guest["accel"].as_str() {
            print!(" / {acc}");
        }
        if let Some(smp) = v.guest["smp"].as_str() {
            print!(" / smp={smp}");
        }
        println!();
    }
    if let Some(kv) = &v.kernel {
        println!(
            "  kernel: {} ({}{})",
            kv.path,
            kv.mtime_unix_ms
                .map(|ms| format!("mtime {}, ", format_utc(ms)))
                .unwrap_or_default(),
            human_size_ls(kv.size_bytes),
        );
    }
    if let Some(q) = &v.qemu_version {
        println!("  qemu: {q}");
    }

    println!(
        "  tests: {} passed / {} failed / {} skipped (reported {}/{}; marker: {})",
        v.summary.pass,
        v.summary.fail,
        v.summary.skip,
        v.summary
            .reported_pass
            .map(|x| x.to_string())
            .unwrap_or_else(|| "?".into()),
        v.summary
            .reported_total
            .map(|x| x.to_string())
            .unwrap_or_else(|| "?".into()),
        v.marker_complete.as_deref().unwrap_or("none"),
    );
    for t in &v.tests {
        println!(
            "    - [{}] {} (asserts pass={} fail={} skip={})",
            t.status.as_str(),
            t.name,
            t.asserts.pass,
            t.asserts.fail,
            t.asserts.skip,
        );
    }
    for line in &v.panics {
        println!("  panic: {line}");
    }
    for line in &v.oops {
        println!("  oops: {line}");
    }

    // 失败时给出串口尾部上下文（超时/panic 定位的第一手证据）。
    if verdict != "passed" {
        let serial = dir.join("serial.log");
        if let Ok(text) = std::fs::read_to_string(&serial) {
            println!("  serial tail (last 15 lines):");
            for line in text
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                println!("    | {line}");
            }
        }
        println!("  repro: {}", v.repro);
    }
    println!("  artifacts: {}", dir.display());
    Ok(if v.verdict == Verdict::Passed { 0 } else { 1 })
}
