//! 分诊命令的终端呈现：triage / runs / cluster / suggest / replay。
//! 聚类与映射语义在 tracker，解析与判定在 judge，本模块只做扫描、IO 与打印。

use std::path::Path;

use anyhow::Context;

use common::fmt::human_size_ls;
use common::time::format_utc;
use judge::report::{AssertCounts, TestEntry};
use judge::Verdict;

use super::rundir::{list_run_dirs, load_verdict_or_parse, resolve_run, runs_root};
use crate::config::Config;

/// `cargo xtask triage`：一次运行的分诊报告。
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

/// `cargo xtask runs`：历史运行列表（最新在前）。
pub fn run_runs(cfg: &Config, json: bool) -> anyhow::Result<i32> {
    let dirs = list_run_dirs(&cfg.project_root);

    if json {
        let items: Vec<serde_json::Value> = dirs
            .iter()
            .map(|d| {
                load_verdict_or_parse(d)
                    .map(|r| serde_json::to_value(r).expect("report to value"))
                    .unwrap_or_else(|e| {
                        serde_json::json!({ "run_id": d.file_name().map(|n| n.to_string_lossy()), "verdict": "unreadable", "error": e.to_string() })
                    })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(items))?
        );
        return Ok(0);
    }
    if dirs.is_empty() {
        println!(
            "no runs yet — cargo xtask test 会写入 {}/",
            runs_root(&cfg.project_root).display()
        );
        return Ok(0);
    }
    println!(
        "{:<22} {:<8} {:<12} {:>5} {:>8}  TESTS",
        "RUN", "ARCH", "VERDICT", "EXIT", "DUR"
    );
    for d in &dirs {
        let (arch, verdict, exit, dur, tests) = match load_verdict_or_parse(d) {
            Ok(r) => (
                r.arch,
                r.verdict.as_str().to_string(),
                r.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into()),
                format!("{:.1}s", r.duration_ms as f64 / 1000.0),
                format!(
                    "{}/{} passed",
                    r.summary.pass,
                    r.summary.reported_total.unwrap_or(0)
                ),
            ),
            Err(_) => (
                "?".into(),
                "?".into(),
                "-".into(),
                "-".into(),
                "0/0 passed".into(),
            ),
        };
        println!(
            "{:<22} {:<8} {:<12} {:>5} {:>8}  {}",
            d.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            arch,
            verdict,
            exit,
            dur,
            tests,
        );
    }
    Ok(0)
}

// ---------------------------------------------------------------- 跨 run 分诊（tracker 呈现层）

fn load_summaries(cfg: &Config) -> anyhow::Result<Vec<tracker::RunSummary>> {
    let dirs = list_run_dirs(&cfg.project_root);
    if dirs.is_empty() {
        anyhow::bail!(
            "未找到运行记录（{}）——先执行一次 cargo xtask test 生成工件",
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

/// `cargo xtask cluster`：跨 run 失败指纹聚类 + flaky 用例清单。
/// 聚类语义在 tracker；此处只做扫描与呈现。
pub fn run_cluster(cfg: &Config, json: bool) -> anyhow::Result<i32> {
    let summaries = load_summaries(cfg)?;
    let clusters = tracker::cluster(&summaries);
    let flaky = tracker::flaky_tests(&summaries);

    if json {
        let out = serde_json::json!({
            "runs_scanned": summaries.len(),
            "clusters": clusters,
            "flaky": flaky,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(0);
    }

    let failed_runs = summaries.iter().filter(|s| s.verdict != "passed").count();
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
            c.verdict,
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
            "  reproduce earliest: cargo xtask triage --run {}",
            first.first_run_id
        );
    }
    Ok(0)
}

/// `cargo xtask suggest`：补丁↔测试映射。--diff 读统一 diff 文件；
/// 缺省对 KERNEL_PATH 内核树执行 git diff（工作区 + 暂存区，文件名去重）。
/// 映射语义在 tracker（DEFAULT_RULES），此处只做 IO 与呈现。
pub fn run_suggest(
    cfg: &Config,
    diff: Option<std::path::PathBuf>,
    json: bool,
) -> anyhow::Result<i32> {
    let changed: Vec<String> = match diff {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 diff {} 失败", path.display()))?;
            tracker::paths_from_unified_diff(&text)
        }
        None => {
            let (kernel_path, _) = cfg.kernel_path()?;
            let mut names: Vec<String> = Vec::new();
            for base in ["HEAD", "--cached"] {
                let out = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&kernel_path)
                    .args(["diff", base, "--name-only"])
                    .output()
                    .context("git 启动失败（内核树需要是 git 仓库）")?;
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    anyhow::bail!("git diff 在 {} 失败: {err}", kernel_path.display());
                }
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let name = line.trim();
                    if !name.is_empty() && !names.iter().any(|x| x == name) {
                        names.push(name.to_string());
                    }
                }
            }
            names
        }
    };

    if changed.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("[SUGGEST] no changed files — nothing to map");
        }
        return Ok(0);
    }

    let suggestions = tracker::suggest_tests(&changed, tracker::DEFAULT_RULES);
    if json {
        println!("{}", serde_json::to_string_pretty(&suggestions)?);
        return Ok(0);
    }

    println!("[SUGGEST] {} changed file(s) mapped", changed.len());
    if suggestions.is_empty() {
        println!("  no subsystem rule matched → full regression:");
        println!("  cargo xtask test --timeout 60   # or: cargo xtask matrix");
    }
    for s in &suggestions {
        println!(
            "  {} ({}): {} file(s)",
            s.matched_prefix,
            s.subsystem,
            s.changed_files.len()
        );
        println!("    minimal set: {}", s.tests.join(", "));
    }
    println!(
        "  run: cargo xtask test --timeout 60 --arch {}",
        cfg.arch()
            .map(|a| a.name().to_string())
            .unwrap_or_else(|| "arm64".into())
    );
    Ok(0)
}

/// `cargo xtask replay` 的 JSON 输出（summary 不含 skip，与 verdict.json 的
/// SummaryCounts 是不同面）。
#[derive(serde::Serialize)]
struct ReplaySummary {
    pass: usize,
    fail: usize,
    reported_pass: Option<u32>,
    reported_total: Option<u32>,
}

#[derive(serde::Serialize)]
struct ReplayReport {
    log: String,
    verdict: Verdict,
    marker_complete: Option<String>,
    summary: ReplaySummary,
    tests: Vec<TestEntry>,
    panics: Vec<String>,
    oops: Vec<String>,
}

/// `cargo xtask replay`：对任意串口日志离线做标记协议断言（不启动 QEMU）。
pub fn run_replay(log: &Path, json: bool) -> anyhow::Result<i32> {
    let text = std::fs::read_to_string(log)
        .with_context(|| format!("读取串口日志 {} 失败", log.display()))?;
    let audit = judge::parse(&text);
    let verdict = judge::judge(0, false, &audit);
    let verdict = if audit.marker.is_none() && audit.panics.is_empty() && audit.oops.is_empty() {
        Verdict::Incomplete
    } else {
        verdict
    };
    if json {
        let report = ReplayReport {
            log: log.display().to_string(),
            verdict,
            marker_complete: audit.marker.clone(),
            summary: ReplaySummary {
                pass: audit.passed_count(),
                fail: audit.failed_count(),
                reported_pass: audit.summary.map(|(a, _)| a),
                reported_total: audit.summary.map(|(_, b)| b),
            },
            tests: audit
                .tests
                .iter()
                .map(|t| TestEntry {
                    name: t.name.clone(),
                    status: t.status,
                    asserts: AssertCounts {
                        pass: t.assert_pass,
                        fail: t.assert_fail,
                        skip: t.assert_skip,
                    },
                })
                .collect(),
            panics: audit.panics.clone(),
            oops: audit.oops.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "[REPLAY] {} — verdict: {}, tests {} passed / {} failed, marker: {}",
            log.display(),
            verdict.as_str(),
            audit.passed_count(),
            audit.failed_count(),
            audit.marker.as_deref().unwrap_or("none"),
        );
        for line in audit.panics.iter().chain(audit.oops.iter()) {
            println!("  ! {line}");
        }
    }
    Ok(if verdict == Verdict::Passed { 0 } else { 1 })
}
