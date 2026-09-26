//! judge — 判定者：串口标记协议 v1 断言引擎、verdict 判定与 verdict.json schema。
//!
//! 协议 v1（冻结文本，设计文档附录 A）：
//! `[PASS]/[FAIL]/[SKIP]/[INFO]` 断言行、`--- Running: X ---` / `PASSED:/FAILED: X`
//! （init 汇编层）、`Test Results: N/M passed` 汇总、`TEST_COMPLETE` 终止标记。
//!
//! 判定语义（Phase 1.5 起）：退出码仍是接口契约，但**不再单独**作为通过依据 ——
//! `-no-reboot` 下内核 panic 会让 QEMU 以 0 退出（假通过）；`judge` 以标记协议
//! 为准并与退出码对账，panic/oops 独立成档。
//!
//! 分节：判定核心（协议类型 + `judge` 对账）→ 退出码语义表（唯一语义表）
//! → 标记协议解析器 → verdict.json schema。

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 判定核心

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TestStatus {
    Pass,
    Fail,
}


#[derive(Debug)]
pub(crate) struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub assert_pass: u32,
    pub assert_fail: u32,
    pub assert_skip: u32,
}

/// events.jsonl 单行的事件信封（schema v2）。`extra` 按 kind 承载各自的
/// 载荷（test_start/test_end: name…；assert: result；summary: passed/total；
/// marker: value；run_end: exit_code/duration_ms/verdict；panic/oops: null）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Event {
    /// 串口日志中的行号（1 起）；run_end 合成事件无行号 → null。
    pub line_no: Option<u64>,
    pub kind: EventKind,
    /// 原始串口行。
    pub text: String,
    /// 出现顺序，从 0 起。
    pub seq: usize,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    TestStart,
    TestEnd,
    Assert,
    Summary,
    Marker,
    Panic,
    Oops,
    RunEnd,
}

/// 一次串口流的整体解析结果。
#[derive(Debug, Default)]
pub(crate) struct Audit {
    pub tests: Vec<TestResult>,
    /// init 报告的 (passed, total)
    pub summary: Option<(u32, u32)>,
    /// TEST_COMPLETE: 之后的内容（None = 协议未走完）
    pub marker: Option<String>,
    pub panics: Vec<String>,
    pub oops: Vec<String>,
    /// 结构化事件流（events.jsonl 的行，按出现顺序）
    pub events: Vec<Event>,
}

/// 运行判定 —— verdict 字符串是冻结接口（runs 工件与 CI 判定依赖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Passed,
    Failed,
    Timeout,
    Panic,
    Incomplete,
    Interrupted,
    BuildFailed,
    Unknown,
}

impl Verdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Verdict::Passed => "passed",
            Verdict::Failed => "failed",
            Verdict::Timeout => "timeout",
            Verdict::Panic => "panic",
            Verdict::Incomplete => "incomplete",
            Verdict::Interrupted => "interrupted",
            Verdict::BuildFailed => "build_failed",
            Verdict::Unknown => "unknown",
        }
    }
}

impl Audit {
    pub(crate) fn passed_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| t.status == TestStatus::Pass)
            .count()
    }

    pub(crate) fn failed_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| t.status == TestStatus::Fail)
            .count()
    }

    pub(crate) fn skipped_count(&self) -> usize {
        self.tests.iter().filter(|t| t.assert_skip > 0).count()
    }

    /// 终止标记对应的 verdict 期望；None = 协议未走完。
    pub(crate) fn marker_verdict(&self) -> Option<bool> {
        match self.marker.as_deref() {
            Some("ALL TESTS PASSED") => Some(true),
            Some("SOME TESTS FAILED") => Some(false),
            _ => None,
        }
    }
}

/// 判定：标记协议与退出码对账。
pub(crate) fn judge(exit_code: i32, build_failed: bool, a: &Audit) -> Verdict {
    if build_failed {
        return Verdict::BuildFailed;
    }
    let kernel_broken = !a.panics.is_empty() || !a.oops.is_empty();
    if exit_code == 0 {
        if kernel_broken {
            Verdict::Panic
        } else {
            match (a.marker_verdict(), a.failed_count()) {
                (Some(true), 0) => Verdict::Passed,
                (None, _) => Verdict::Incomplete,
                _ => Verdict::Failed,
            }
        }
    } else if (exit_code == 124 || exit_code == 137) && kernel_broken {
        Verdict::Panic
    } else if exit_code == 130 {
        Verdict::Interrupted
    } else if exit_code == 124 || exit_code == 137 {
        Verdict::Timeout
    } else {
        Verdict::Failed
    }
}

// ---------------------------------------------------------------- 退出码语义表

// 退出码语义 —— 唯一的语义表（冻结：0=通过、124=超时、137 归一为 124、
// 130=中断；其余非零=失败）。`judge` 的对账、host 侧收割与 verdict.json 的
// `exit_semantics` 字段都从这里取值，不允许各自再写一份映射。

/// 超时退出码（与 `timeout --signal=KILL` 的 124 语义一致）。
pub(crate) const EXIT_TIMEOUT: i32 = 124;
/// Ctrl-C / 信号中断退出码（常量钉在 crate::util —— guardian 不依赖 judge）。
pub(crate) use crate::util::EXIT_INTERRUPTED;

/// 退出码归一：137（SIGKILL）按超时语义归一为 124；无退出码（信号死亡）
/// 归一为 130；其余保留真实码。
pub(crate) fn normalize(code: Option<i32>) -> i32 {
    match code {
        Some(124) | Some(137) => EXIT_TIMEOUT,
        Some(n) => n,
        None => EXIT_INTERRUPTED,
    }
}

/// 退出码 → 语义标签（verdict.json 的 `exit_semantics` 字段）。
pub(crate) fn semantics(code: Option<i32>) -> &'static str {
    match code {
        Some(0) => "ok",
        Some(124) => "timeout",
        Some(130) => "interrupted",
        Some(_) => "error",
        None => "unknown",
    }
}

// ---------------------------------------------------------------- 标记协议解析器

// 标记协议 v1 解析器：串口全文 → 结构化 Audit（tests/summary/marker/
// panics/oops + events 事件流）。协议冻结文本见 docs/architecture/contracts.md。

/// 解析串口全文。行号从 1 起；事件含 line_no 与出现顺序 seq。
pub(crate) fn parse(text: &str) -> Audit {
    let mut a = Audit::default();
    let mut current: Option<usize> = None;

    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let t = raw.trim_start();

        let ev = |kind: EventKind, extra: serde_json::Value, a: &mut Audit| {
            let seq = a.events.len();
            a.events.push(Event {
                line_no: Some(line_no as u64),
                kind,
                text: raw.to_string(),
                seq,
                extra,
            });
        };

        if let Some(name) = t
            .strip_prefix("--- Running: ")
            .and_then(|s| s.strip_suffix(" ---"))
        {
            current = Some(a.tests.len());
            a.tests.push(TestResult {
                name: name.trim().to_string(),
                status: TestStatus::Fail,
                assert_pass: 0,
                assert_fail: 0,
                assert_skip: 0,
            });
            ev(
                EventKind::TestStart,
                serde_json::json!({ "name": name.trim() }),
                &mut a,
            );
        } else if let Some(name) = t.strip_prefix("PASSED: ") {
            if let Some(tr) = a.tests.iter_mut().rev().find(|tr| tr.name == name.trim()) {
                tr.status = TestStatus::Pass;
            }
            ev(
                EventKind::TestEnd,
                serde_json::json!({ "name": name.trim(), "status": "pass" }),
                &mut a,
            );
        } else if let Some(name) = t.strip_prefix("FAILED: ") {
            if let Some(tr) = a.tests.iter_mut().rev().find(|tr| tr.name == name.trim()) {
                tr.status = TestStatus::Fail;
            }
            ev(
                EventKind::TestEnd,
                serde_json::json!({ "name": name.trim(), "status": "fail" }),
                &mut a,
            );
        } else if t.starts_with("[PASS] ") {
            if let Some(idx) = current {
                if let Some(tr) = a.tests.get_mut(idx) {
                    tr.assert_pass += 1;
                }
            }
            ev(
                EventKind::Assert,
                serde_json::json!({ "result": "pass" }),
                &mut a,
            );
        } else if t.starts_with("[FAIL] ") {
            if let Some(idx) = current {
                if let Some(tr) = a.tests.get_mut(idx) {
                    tr.assert_fail += 1;
                }
            }
            ev(
                EventKind::Assert,
                serde_json::json!({ "result": "fail" }),
                &mut a,
            );
        } else if t.starts_with("[SKIP] ") {
            if let Some(idx) = current {
                if let Some(tr) = a.tests.get_mut(idx) {
                    tr.assert_skip += 1;
                }
            }
            ev(
                EventKind::Assert,
                serde_json::json!({ "result": "skip" }),
                &mut a,
            );
        } else if let Some(rest) = t.strip_prefix("Test Results: ") {
            let frac = rest.split_whitespace().find(|tok| tok.contains('/'));
            let pair = frac.and_then(|f| {
                let (x, y) = f.split_once('/')?;
                Some((x.parse::<u32>().ok()?, y.parse::<u32>().ok()?))
            });
            if let Some((passed, total)) = pair {
                a.summary = Some((passed, total));
                ev(
                    EventKind::Summary,
                    serde_json::json!({ "passed": passed, "total": total }),
                    &mut a,
                );
            }
        } else if let Some(rest) = t.strip_prefix("TEST_COMPLETE: ") {
            a.marker = Some(rest.trim().to_string());
            ev(
                EventKind::Marker,
                serde_json::json!({ "value": rest.trim() }),
                &mut a,
            );
        } else if raw.contains("Kernel panic - not syncing") {
            a.panics.push(raw.to_string());
            ev(EventKind::Panic, serde_json::Value::Null, &mut a);
        } else if raw.contains("Oops:") {
            a.oops.push(raw.to_string());
            ev(EventKind::Oops, serde_json::Value::Null, &mut a);
        }
    }
    a
}

// ---------------------------------------------------------------- verdict.json schema

// verdict.json 的 schema 与构造（schema 单一来源）：serde 字段序即落盘
// 键序；构造（`VerdictReport::build`）与回读（serde 反序列化）共用同一
// 结构体，键名/取值是编译期保证。IO（落盘、目录管理）在 runs 层，这里只
// 管语义。

/// verdict.json 顶层 schema（schema: 2）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct VerdictReport {
    pub schema: u32,
    pub run_id: String,
    pub arch: String,
    pub started_at_unix_ms: Option<u64>,
    pub started_at_utc: Option<String>,
    pub duration_ms: u64,
    pub timeout_s: u64,
    pub exit_code: Option<i32>,
    pub exit_semantics: String,
    pub verdict: Verdict,
    pub kernel: Option<FileFingerprint>,
    pub qemu_version: Option<String>,
    pub guest: serde_json::Value,
    pub tests: Vec<TestEntry>,
    pub summary: SummaryCounts,
    pub marker_complete: Option<String>,
    pub panics: Vec<String>,
    pub oops: Vec<String>,
    pub repro: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Artifacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict_source: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileFingerprint {
    pub path: String,
    pub size_bytes: u64,
    pub mtime_unix_ms: Option<u64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TestEntry {
    pub name: String,
    pub status: TestStatus,
    pub asserts: AssertCounts,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct AssertCounts {
    pub pass: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct SummaryCounts {
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
    pub reported_pass: Option<u32>,
    pub reported_total: Option<u32>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Artifacts {
    pub serial: String,
    pub qemu_stderr: String,
    pub build_log: String,
    pub events: String,
}

/// 一次运行的元数据（verdict.json 的非解析部分）。
pub(crate) struct RunMeta {
    pub run_id: String,
    pub arch: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timeout_s: String,
    pub kernel: Option<PathBuf>,
    pub qemu_version: Option<String>,
    pub topo: serde_json::Value,
    pub build_failed: bool,
}

impl RunMeta {
    fn started_ms(&self) -> Option<u64> {
        self.run_id.split('-').next().and_then(|s| s.parse().ok())
    }

    fn exit_semantics(&self) -> &'static str {
        semantics(self.exit_code)
    }

    fn repro(&self) -> String {
        format!(
            "virtuoso test --timeout {} --arch {}",
            self.timeout_s, self.arch
        )
    }

    pub(crate) fn verdict_of(&self, audit: &Audit) -> Verdict {
        if self.build_failed {
            Verdict::BuildFailed
        } else {
            judge(self.exit_code.unwrap_or(-1), false, audit)
        }
    }
}

fn file_fingerprint(p: &Path) -> Option<FileFingerprint> {
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    Some(FileFingerprint {
        path: p.display().to_string(),
        size_bytes: meta.len(),
        mtime_unix_ms: mtime,
    })
}

impl VerdictReport {
    /// verdict.json 的唯一构造处：judge 解析 + 运行元数据 → 报告。
    pub(crate) fn build(audit: &Audit, meta: &RunMeta) -> VerdictReport {
        VerdictReport {
            schema: 2,
            run_id: meta.run_id.clone(),
            arch: meta.arch.clone(),
            started_at_unix_ms: meta.started_ms(),
            started_at_utc: meta.started_ms().map(crate::util::format_utc),
            duration_ms: meta.duration_ms,
            timeout_s: meta.timeout_s.parse().unwrap_or(0),
            exit_code: meta.exit_code,
            exit_semantics: meta.exit_semantics().to_string(),
            verdict: meta.verdict_of(audit),
            kernel: meta.kernel.as_deref().and_then(file_fingerprint),
            qemu_version: meta.qemu_version.clone(),
            guest: meta.topo.clone(),
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
            summary: SummaryCounts {
                pass: audit.passed_count(),
                fail: audit.failed_count(),
                skip: audit.skipped_count(),
                reported_pass: audit.summary.map(|(a, _)| a),
                reported_total: audit.summary.map(|(_, b)| b),
            },
            marker_complete: audit.marker.clone(),
            panics: audit.panics.clone(),
            oops: audit.oops.clone(),
            repro: meta.repro(),
            artifacts: None,
            verdict_source: None,
        }
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_passing_run() {
        let log = "--- Running: test_a ---\n  [PASS] ok\nPASSED: test_a\n\
                   Test Results: 1/1 passed\nTEST_COMPLETE: ALL TESTS PASSED\n";
        let a = parse(log);
        assert_eq!(a.tests.len(), 1);
        assert_eq!(a.tests[0].status, TestStatus::Pass);
        assert_eq!(a.tests[0].assert_pass, 1);
        assert_eq!(a.summary, Some((1, 1)));
        assert_eq!(a.marker_verdict(), Some(true));
        assert_eq!(judge(0, false, &a), Verdict::Passed);
    }

    #[test]
    fn parses_failure_with_skip_counts() {
        let log = "--- Running: t1 ---\n  [PASS] a\n  [FAIL] b\n  [SKIP] c\nFAILED: t1\n\
                   --- Running: t2 ---\nPASSED: t2\n\
                   Test Results: 1/2 passed\nTEST_COMPLETE: SOME TESTS FAILED\n";
        let a = parse(log);
        assert_eq!(a.tests[0].status, TestStatus::Fail);
        assert_eq!(a.tests[0].assert_fail, 1);
        assert_eq!(a.tests[0].assert_skip, 1);
        assert_eq!(a.tests[1].status, TestStatus::Pass);
        assert_eq!(a.summary, Some((1, 2)));
        assert_eq!(judge(1, false, &a), Verdict::Failed);
        assert_eq!(
            judge(0, false, &a),
            Verdict::Failed,
            "marker says failed → failed even with exit 0"
        );
    }

    #[test]
    fn summary_fraction_parses_exactly_two_numbers() {
        let a = parse("Test Results: 3/7 passed\n");
        assert_eq!(a.summary, Some((3, 7)));
        let b = parse("Test Results: passed\n");
        assert_eq!(b.summary, None);
    }

    #[test]
    fn panic_with_exit_zero_is_false_pass_detected() {
        // -no-reboot：panic 触发 reset → QEMU exit 0，无 TEST_COMPLETE
        let log = "--- Running: t1 ---\n  [PASS] step\n\
                   [    1.2] Kernel panic - not syncing: boom\n";
        let a = parse(log);
        assert_eq!(a.panics.len(), 1);
        assert_eq!(judge(0, false, &a), Verdict::Panic);
    }

    #[test]
    fn timeout_with_panic_is_panic_not_timeout() {
        let a = parse("Kernel panic - not syncing: boom\n");
        assert_eq!(judge(124, false, &a), Verdict::Panic);
        assert_eq!(judge(137, false, &a), Verdict::Panic);
    }

    #[test]
    fn timeout_without_marker_is_timeout() {
        let a = parse("[    0.0] Linux version 6.6.0\n");
        assert_eq!(judge(124, false, &a), Verdict::Timeout);
    }

    #[test]
    fn missing_marker_with_exit_zero_is_incomplete() {
        let a = parse("--- Running: t1 ---\nPASSED: t1\n");
        assert_eq!(judge(0, false, &a), Verdict::Incomplete);
    }

    #[test]
    fn oops_alone_blocks_pass() {
        let log = "Oops: 0000 [#1]\nTEST_COMPLETE: ALL TESTS PASSED\n";
        let a = parse(log);
        assert_eq!(judge(0, false, &a), Verdict::Panic);
    }

    #[test]
    fn build_failure_short_circuits() {
        let a = parse("TEST_COMPLETE: ALL TESTS PASSED\n");
        assert_eq!(judge(0, true, &a), Verdict::BuildFailed);
    }

    #[test]
    fn events_carry_line_numbers_and_order() {
        let a =
            parse("--- Running: t ---\n  [FAIL] x\nFAILED: t\nTEST_COMPLETE: SOME TESTS FAILED\n");
        let kinds: Vec<EventKind> = a.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                EventKind::TestStart,
                EventKind::Assert,
                EventKind::TestEnd,
                EventKind::Marker
            ]
        );
        assert_eq!(a.events[0].line_no, Some(1));
        assert_eq!(a.events[1].line_no, Some(2));
        assert_eq!(a.events[0].seq, 0);
        assert_eq!(a.events[3].seq, 3);
        assert_eq!(a.events[2].text, "FAILED: t");
    }

    #[test]
    fn events_roundtrip_through_jsonl() {
        // events.jsonl 行必须能原样回读（schema 冻结的回归网）
        let a = parse("Kernel panic - not syncing: boom\n");
        let line = serde_json::to_string(&a.events[0]).expect("serialize");
        let back: Event = serde_json::from_str(&line).expect("deserialize");
        assert_eq!(back.kind, EventKind::Panic);
        assert_eq!(back.line_no, Some(1));
        assert_eq!(back.extra, serde_json::Value::Null);
    }

    #[test]
    fn normalize_preserves_real_codes() {
        assert_eq!(normalize(Some(0)), 0);
        assert_eq!(normalize(Some(1)), 1);
        assert_eq!(normalize(Some(124)), 124);
        assert_eq!(normalize(Some(137)), 124, "SIGKILL 按超时语义归一");
        assert_eq!(normalize(None), 130);
    }

    #[test]
    fn semantics_labels() {
        assert_eq!(semantics(Some(0)), "ok");
        assert_eq!(semantics(Some(124)), "timeout");
        assert_eq!(semantics(Some(130)), "interrupted");
        assert_eq!(semantics(Some(1)), "error");
        assert_eq!(semantics(None), "unknown");
    }
}
