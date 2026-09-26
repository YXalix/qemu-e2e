//! 分诊命令的终端呈现：triage / cluster。
//! 聚类语义在 tracker，解析与判定在 judge，本模块只做扫描、IO 与打印。

use common::fmt::human_size_ls;
use common::time::format_utc;
use judge::Verdict;

use super::rundir::{list_run_dirs, load_verdict_or_parse, resolve_run, runs_root};
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

// ---------------------------------------------------------------- 跨 run 分诊（tracker 呈现层）

fn load_summaries(cfg: &Config) -> anyhow::Result<Vec<tracker::RunSummary>> {
    let dirs = list_run_dirs(&cfg.project_root);
    if dirs.is_empty() {
        anyhow::bail!(
            "no runs found ({}) — run virtuoso test once to produce artifacts",
            runs_root(&cfg.project_root).display()
        );
    }
    Ok(dirs
        .into_iter()
        .filter_map(|d| {
            load_verdict_or_parse(&d)
                .ok()
                .map(tracker::RunSummary::from)
        })
        .collect())
}

/// `virtuoso cluster`：跨 run 失败指纹聚类 + flaky 用例清单。
/// 聚类语义在 tracker；此处只做扫描与呈现。
pub fn run_cluster(cfg: &Config, json: bool) -> anyhow::Result<i32> {
    let summaries = load_summaries(cfg)?;
    let clusters = tracker::cluster(&summaries);
    let flaky = tracker::flaky_tests(&summaries);

    let failed_runs = summaries
        .iter()
        .filter(|s| s.verdict != Verdict::Passed)
        .count();
    if json {
        let out = serde_json::json!({
            "runs_scanned": summaries.len(),
            "failed_runs": failed_runs,
            "clusters": clusters,
            "flaky": flaky,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(i32::from(failed_runs > 0));
    }
    println!(
        "[CLUSTER] {} runs scanned, {} failed, {} fingerprint bucket(s)",
        summaries.len(),
        failed_runs,
        clusters.len()
    );
    if clusters.is_empty() {
        println!("  no failure fingerprints — all runs passed");
    }
    for (i, c) in clusters.iter().enumerate() {
        println!(
            "  #{} x{} [{}] {}",
            i + 1,
            c.count,
            c.verdict.as_str(),
            if c.key.len() > 120 {
                format!("{}…", &c.key[..120])
            } else {
                c.key.clone()
            }
        );
        println!(
            "           first: {}  last: {}  arches: {}",
            c.first_run_id,
            c.last_run_id,
            c.arches.join(",")
        );
    }
    println!();
    if flaky.is_empty() {
        println!("  flaky: none");
    } else {
        println!("  flaky: {} test(s)", flaky.len());
        for f in &flaky {
            println!(
                "    - {} passed in [{}] / failed in [{}]",
                f.test,
                f.passed_in.join(","),
                f.failed_in.join(",")
            );
        }
    }
    if let Some(first) = clusters.first() {
        println!(
            "  reproduce earliest: virtuoso triage --run {}",
            first.first_run_id
        );
    }
    Ok(i32::from(failed_runs > 0))
}
