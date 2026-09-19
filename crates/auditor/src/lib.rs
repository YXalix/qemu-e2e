//! auditor — 听审：串口监听与标记协议 v1 断言引擎。
//! Phase 2 接管串口 expect（rexpect/pty）：[PASS]/[FAIL]/[SKIP]/[INFO]、
//! "Test Results: N/M passed"、TEST_COMPLETE 终止标记解析，
//! 并产出 events.jsonl 结构化事件流（AI triage 的数据源）。
