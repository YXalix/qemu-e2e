//! 标记协议 v1 解析器：串口全文 → 结构化 Audit（tests/summary/marker/
//! panics/oops + events 事件流）。协议冻结文本见 docs/architecture/contracts.md。

use crate::{Audit, Event, EventKind, TestResult, TestStatus};

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
