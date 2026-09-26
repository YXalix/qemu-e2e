//! judge — 判定者：串口标记协议 v1 断言引擎、verdict 判定与 verdict.json schema。
//!
//! 协议 v1（冻结文本，设计文档附录 A）：
//! `[PASS]/[FAIL]/[SKIP]/[INFO]` 断言行、`--- Running: X ---` / `PASSED:/FAILED: X`
//! （init 汇编层）、`Test Results: N/M passed` 汇总、`TEST_COMPLETE` 终止标记。
//!
//! 判定语义（Phase 1.5 起）：退出码仍是接口契约，但**不再单独**作为通过依据 ——
//! `-no-reboot` 下内核 panic 会让 QEMU 以 0 退出（假通过）；`judge` 以标记协议
//! 为准并与退出码对账，panic/oops 独立成档。

pub mod exit;
pub mod report;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Pass,
    Fail,
}

impl TestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TestStatus::Pass => "pass",
            TestStatus::Fail => "fail",
        }
    }
}

#[derive(Debug)]
pub struct TestResult {
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
pub struct Event {
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
pub enum EventKind {
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
pub struct Audit {
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

/// 运行判定 —— verdict 字符串是冻结接口（triage/runs/CI 三态判定依赖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
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
    pub fn as_str(self) -> &'static str {
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

/// 解析串口全文。行号从 1 起；事件含 line_no 与出现顺序 seq。
pub fn parse(text: &str) -> Audit {
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

impl Audit {
    pub fn passed_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| t.status == TestStatus::Pass)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| t.status == TestStatus::Fail)
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.tests.iter().filter(|t| t.assert_skip > 0).count()
    }

    /// 终止标记对应的 verdict 期望；None = 协议未走完。
    pub fn marker_verdict(&self) -> Option<bool> {
        match self.marker.as_deref() {
            Some("ALL TESTS PASSED") => Some(true),
            Some("SOME TESTS FAILED") => Some(false),
            _ => None,
        }
    }
}

/// 判定：标记协议与退出码对账。
pub fn judge(exit_code: i32, build_failed: bool, a: &Audit) -> Verdict {
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
}
