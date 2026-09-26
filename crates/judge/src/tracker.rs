//! tracker — 跨 run 分诊语义：失败指纹聚类与 flaky 识别。
//!
//! 职责：对**最小化运行摘要**（`RunSummary`）做失败指纹聚类与 flaky 识别。
//! 本模块只承载语义；IO 与呈现（runs 目录扫描、报告打印）在 cli 的 runs 层
//! —— verdict.json schema 由 `crate::report::VerdictReport` 单点定义，
//! `RunSummary` 从它 `From` 投影而来。
//!
//! 输入刻意收敛为 `RunSummary`（而非 verdict.json 全文），使聚类规则与
//! schema 演进解耦。

use serde::Serialize;

// ---------------------------------------------------------------- 输入摘要

/// 一次运行的最小摘要（由 `crate::report::VerdictReport` 投影而来）。
/// verdict/status 直接复用 judge 的类型化枚举（serde 形状 = 冻结字符串），
/// 魔法串比较在编译期对齐 judge 语义。
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub arch: String,
    pub verdict: crate::Verdict,
    /// (test_name, status)
    pub tests: Vec<(String, crate::TestStatus)>,
    pub panics: Vec<String>,
    pub oops: Vec<String>,
}

impl From<crate::report::VerdictReport> for RunSummary {
    fn from(report: crate::report::VerdictReport) -> Self {
        RunSummary {
            run_id: report.run_id,
            arch: report.arch,
            verdict: report.verdict,
            tests: report
                .tests
                .into_iter()
                .map(|t| (t.name, t.status))
                .collect(),
            panics: report.panics,
            oops: report.oops,
        }
    }
}

// ---------------------------------------------------------------- 指纹

/// 失败指纹：verdict 类 + 归一化证据。`None` = 通过/未知结果的运行（无需指纹）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fingerprint {
    pub verdict: crate::Verdict,
    pub key: String,
}

/// 归一化一行内核证据：剥离时间戳 `[ 123.4567]`、把连续数字折叠为 `N`
/// （PID、地址、计数抖动不影响指纹，同一根因落在同一桶）。
pub fn normalize_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        // 剥离 "[ 123.4567]" 形态的内核时间戳
        if c == '[' {
            let rest = &line[i..];
            if let Some(close) = rest.find(']') {
                let inner = &rest[1..close];
                let trimmed = inner.trim();
                let looks_like_ts = trimmed.contains('.')
                    && trimmed
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == ' ');
                if looks_like_ts {
                    for _ in 0..close {
                        chars.next();
                    }
                    // 吞掉时间戳后紧跟的空白，保证指纹不含对齐抖动
                    while chars.peek().is_some_and(|(_, ch)| *ch == ' ') {
                        chars.next();
                    }
                    continue;
                }
            }
        }
        if c.is_ascii_digit() {
            out.push('N');
            while chars.peek().is_some_and(|(_, ch)| ch.is_ascii_digit()) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 提取失败指纹。规则：
/// - panic/oops 在场 → 归一化的首条 panic/oops 行（内核级根因优先级最高）；
/// - timeout → "timeout"（timeout + panic 已被上一条接住）；
/// - failed → 归一化失败的测试名集合（排序，跨 run 可比）；
/// - 其余（incomplete/interrupted/build_failed）→ 仅 verdict 类。
pub fn fingerprint_of(run: &RunSummary) -> Option<Fingerprint> {
    use crate::Verdict;
    if run.verdict == Verdict::Passed || run.verdict == Verdict::Unknown {
        return None;
    }
    let key = if let Some(p) = run.panics.first() {
        format!("panic: {}", normalize_line(p))
    } else if let Some(o) = run.oops.first() {
        format!("oops: {}", normalize_line(o))
    } else if run.verdict == Verdict::Timeout {
        "timeout".to_string()
    } else if run.verdict == Verdict::Failed {
        let mut names: Vec<String> = run
            .tests
            .iter()
            .filter(|(_, s)| *s == crate::TestStatus::Fail)
            .map(|(n, _)| normalize_line(n))
            .collect();
        if names.is_empty() {
            names = run.tests.iter().map(|(n, _)| normalize_line(n)).collect();
        }
        names.sort();
        format!("failed tests: {}", names.join(", "))
    } else {
        run.verdict.as_str().to_string()
    };
    Some(Fingerprint {
        verdict: run.verdict,
        key,
    })
}

// ---------------------------------------------------------------- 聚类

/// 同一指纹的历史聚合桶。
#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    pub verdict: crate::Verdict,
    pub key: String,
    /// 指纹首现（run_id 前缀是定宽 unix_ms，min/max 即首现/末现）
    pub first_run_id: String,
    pub last_run_id: String,
    pub count: usize,
    pub arches: Vec<String>,
    pub run_ids: Vec<String>,
}

/// 跨 run 聚类：按指纹分桶，桶内记录首现/末现/架构分布。
/// 输出按 count 降序、key 升序（稳定呈现）。
pub fn cluster(runs: &[RunSummary]) -> Vec<Cluster> {
    let mut buckets: Vec<Cluster> = Vec::new();
    for run in runs {
        let Some(fp) = fingerprint_of(run) else {
            continue;
        };
        if let Some(c) = buckets.iter_mut().find(|c| c.key == fp.key) {
            c.count += 1;
            c.run_ids.push(run.run_id.clone());
            if run.run_id < c.first_run_id {
                c.first_run_id = run.run_id.clone();
            }
            if run.run_id > c.last_run_id {
                c.last_run_id = run.run_id.clone();
            }
            if !c.arches.contains(&run.arch) {
                c.arches.push(run.arch.clone());
            }
        } else {
            buckets.push(Cluster {
                verdict: fp.verdict,
                key: fp.key,
                first_run_id: run.run_id.clone(),
                last_run_id: run.run_id.clone(),
                count: 1,
                arches: vec![run.arch.clone()],
                run_ids: vec![run.run_id.clone()],
            });
        }
    }
    buckets.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
    buckets
}

// ---------------------------------------------------------------- flaky

/// flaky 用例：同一测试名在历史中既通过又失败（跨 run 不稳定）。
#[derive(Debug, Clone, Serialize)]
pub struct Flaky {
    pub test: String,
    pub passed_in: Vec<String>,
    pub failed_in: Vec<String>,
}

pub fn flaky_tests(runs: &[RunSummary]) -> Vec<Flaky> {
    use crate::{TestStatus, Verdict};
    let mut passed: Vec<(String, Vec<String>)> = Vec::new();
    let mut failed: Vec<(String, Vec<String>)> = Vec::new();
    for run in runs {
        // 只有协议走完的 run 才可信：incomplete 的测试条目不参与 flaky 判定
        let trustworthy = run.verdict == Verdict::Passed || run.verdict == Verdict::Failed;
        if !trustworthy {
            continue;
        }
        for (name, status) in &run.tests {
            let bucket = if *status == TestStatus::Pass {
                &mut passed
            } else {
                &mut failed
            };
            match bucket.iter_mut().find(|(n, _)| n == name) {
                Some((_, ids)) => {
                    if !ids.contains(&run.run_id) {
                        ids.push(run.run_id.clone());
                    }
                }
                None => bucket.push((name.clone(), vec![run.run_id.clone()])),
            }
        }
    }
    let mut out: Vec<Flaky> = passed
        .into_iter()
        .filter_map(|(name, passed_in)| {
            failed
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, failed_in)| Flaky {
                    test: name,
                    passed_in,
                    failed_in: failed_in.clone(),
                })
        })
        .collect();
    out.sort_by(|a, b| a.test.cmp(&b.test));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, verdict: crate::Verdict, tests: &[(&str, crate::TestStatus)]) -> RunSummary {
        RunSummary {
            run_id: id.into(),
            arch: "arm64".into(),
            verdict,
            tests: tests
                .to_vec()
                .iter()
                .map(|(n, s)| (n.to_string(), *s))
                .collect(),
            panics: vec![],
            oops: vec![],
        }
    }

    use crate::{TestStatus as TS, Verdict as V};

    #[test]
    fn normalize_strips_timestamps_and_folds_numbers() {
        let n = normalize_line("[  100.5] Oops: 0000 [#1] PREEMPT SMP");
        assert_eq!(n, "Oops: N [#N] PREEMPT SMP");
        let p = normalize_line("Kernel panic - not syncing: rig for pid=99 at 0xdead0000");
        assert_eq!(p, "Kernel panic - not syncing: rig for pid=N at NxdeadN");
    }

    #[test]
    fn normalize_keeps_brackets_without_timestamp_shape() {
        assert_eq!(normalize_line("[PASS] ok"), "[PASS] ok");
        assert_eq!(normalize_line("[ 101 ] tag"), "[ N ] tag");
    }

    #[test]
    fn fingerprint_ignores_passing_runs() {
        assert!(fingerprint_of(&run("1", V::Passed, &[("t", TS::Pass)])).is_none());
        assert!(fingerprint_of(&run("1", V::Unknown, &[])).is_none());
    }

    #[test]
    fn fingerprint_timeout_and_failed_tests() {
        let fp = fingerprint_of(&run("r1", V::Timeout, &[])).unwrap();
        assert_eq!(fp.key, "timeout");

        let fp = fingerprint_of(&run(
            "r2",
            V::Failed,
            &[("t_a", TS::Fail), ("t_ok", TS::Pass), ("t_b", TS::Fail)],
        ))
        .unwrap();
        assert_eq!(fp.key, "failed tests: t_a, t_b");
    }

    #[test]
    fn fingerprint_panic_overrides_and_normalizes() {
        let mut r = run("r", V::Panic, &[("t", TS::Pass)]);
        r.panics = vec!["[   12.345678] Kernel panic - not syncing: rig for pid=99".into()];
        let fp = fingerprint_of(&r).unwrap();
        assert_eq!(fp.key, "panic: Kernel panic - not syncing: rig for pid=N");
    }

    #[test]
    fn cluster_buckets_by_key_and_tracks_first_seen() {
        let runs = vec![
            run("0002-a", V::Timeout, &[]),
            run("0001-a", V::Timeout, &[]),
            run("0003-b", V::Passed, &[("t", TS::Pass)]),
        ];
        let cs = cluster(&runs);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].count, 2);
        assert_eq!(cs[0].first_run_id, "0001-a");
        assert_eq!(cs[0].last_run_id, "0002-a");
    }

    #[test]
    fn cluster_orders_by_count_desc() {
        let r1 = run("a", V::Failed, &[("x", TS::Fail)]);
        let r2 = run("b", V::Failed, &[("x", TS::Fail)]);
        let r3 = run("c", V::Failed, &[("y", TS::Fail)]);
        let cs = cluster(&[r1, r2, r3]);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].count, 2);
        assert!(cs[0].key.contains("x"));
    }

    #[test]
    fn flaky_detects_unstable_tests_only_from_trustworthy_runs() {
        let runs = vec![
            run("1", V::Passed, &[("t", TS::Pass)]),
            run("2", V::Failed, &[("t", TS::Fail)]),
            run("3", V::Incomplete, &[("t", TS::Fail)]), // 不可信，不参与
        ];
        let flaky = flaky_tests(&runs);
        assert_eq!(flaky.len(), 1);
        assert_eq!(flaky[0].test, "t");
        assert_eq!(flaky[0].passed_in, vec!["1".to_string()]);
        assert_eq!(flaky[0].failed_in, vec!["2".to_string()]);
    }

    #[test]
    fn stable_test_is_not_flaky() {
        let runs = vec![
            run("1", V::Passed, &[("t", TS::Pass)]),
            run("2", V::Passed, &[("t", TS::Pass)]),
        ];
        assert!(flaky_tests(&runs).is_empty());
    }
}
