//! verdict.json 的 schema 与构造（schema 单一来源）。
//!
//! serde 字段序即落盘键序；构造（`VerdictReport::build`）与回读
//! （serde 反序列化）共用同一结构体，键名/取值是编译期保证。
//! IO（落盘、目录管理）在 xtask 的 runs 层，本模块只管语义。

use std::path::Path;

use crate::{Audit, Verdict};

/// verdict.json 顶层 schema（schema: 2）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VerdictReport {
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
pub struct FileFingerprint {
    pub path: String,
    pub size_bytes: u64,
    pub mtime_unix_ms: Option<u64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TestEntry {
    pub name: String,
    pub status: crate::TestStatus,
    pub asserts: AssertCounts,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AssertCounts {
    pub pass: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SummaryCounts {
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
    pub reported_pass: Option<u32>,
    pub reported_total: Option<u32>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Artifacts {
    pub serial: String,
    pub qemu_stderr: String,
    pub build_log: String,
    pub events: String,
}

/// 一次运行的元数据（verdict.json 的非解析部分）。
pub struct RunMeta {
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

use std::path::PathBuf;

impl RunMeta {
    fn started_ms(&self) -> Option<u64> {
        self.run_id.split('-').next().and_then(|s| s.parse().ok())
    }

    fn exit_semantics(&self) -> &'static str {
        crate::exit::semantics(self.exit_code)
    }

    fn repro(&self) -> String {
        format!(
            "cargo xtask test --timeout {} --arch {}",
            self.timeout_s, self.arch
        )
    }

    pub fn verdict_of(&self, audit: &Audit) -> Verdict {
        if self.build_failed {
            Verdict::BuildFailed
        } else {
            crate::judge(self.exit_code.unwrap_or(-1), false, audit)
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
    pub fn build(audit: &Audit, meta: &RunMeta) -> VerdictReport {
        VerdictReport {
            schema: 2,
            run_id: meta.run_id.clone(),
            arch: meta.arch.clone(),
            started_at_unix_ms: meta.started_ms(),
            started_at_utc: meta.started_ms().map(common::time::format_utc),
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
