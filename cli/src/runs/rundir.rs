//! 运行目录与工件 IO：目录生命周期、输出泵、verdict.json 落盘/回读。

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};

use anyhow::Context;

use common::time::unix_ms;
use judge::report::{Artifacts, RunMeta, VerdictReport};
use judge::{Audit, Event, EventKind};

pub const RUNS_KEEP: usize = 20;

// ---------------------------------------------------------------- 运行目录

pub fn runs_root(project_root: &Path) -> PathBuf {
    project_root.join("target/runs")
}

pub struct RunDir {
    pub id: String,
    pub path: PathBuf,
}

/// 以 `<unix_ms>-<arch>` 建目录（定宽时间戳，目录名排序即时间排序）。
pub fn create_run_dir(project_root: &Path, arch: &str) -> anyhow::Result<RunDir> {
    let id = format!("{}-{arch}", unix_ms());
    let path = runs_root(project_root).join(&id);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("创建运行目录 {} 失败", path.display()))?;
    Ok(RunDir { id, path })
}

/// 解析 `--run`：None/"latest" → 最新一次；否则为 runs 下的目录名（拒绝路径穿越）。
pub fn resolve_run(project_root: &Path, spec: Option<&str>) -> anyhow::Result<PathBuf> {
    let root = runs_root(project_root);
    let path = match spec.filter(|s| !s.is_empty() && *s != "latest") {
        None => latest_run(project_root),
        Some(name) => {
            if name.contains('/') || name.contains("..") {
                anyhow::bail!("非法 run 标识: {name}");
            }
            let p = root.join(name);
            p.is_dir().then_some(p)
        }
    };
    path.ok_or_else(|| {
        anyhow::anyhow!(
            "未找到运行记录（{}）——先执行一次 virtuoso test 生成工件",
            root.display()
        )
    })
}

pub fn latest_run(project_root: &Path) -> Option<PathBuf> {
    list_run_dirs(project_root).into_iter().next()
}

pub fn list_run_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(runs_root(project_root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs
}

/// 只保留最近 `keep` 次运行（目录名定宽时间戳，字典序即时间序）。
pub fn prune(project_root: &Path, keep: usize) {
    for stale in list_run_dirs(project_root).into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(stale);
    }
}

// ---------------------------------------------------------------- 输出泵

/// 失败时回显的 QEMU stderr 尾部行数上限。
pub const QEMU_TAIL_LINES: usize = 40;

/// 两条输出流的分离呈现：guest 串口（stdout）逐行回显终端 + 落 serial.log；
/// QEMU 自身输出（stderr）只落 qemu-stderr.log，不刷屏，仅保留尾部摘录，
/// 运行失败时回显（QEMU 早夭/参数被拒的现场就在这几行里）。
#[derive(Debug)]
pub struct PumpOutcome {
    pub status: ExitStatus,
    /// QEMU stderr 总行数。
    pub qemu_lines: usize,
    /// 末尾至多 `QEMU_TAIL_LINES` 行（不含换行）。
    pub qemu_tail: Vec<String>,
}

impl PumpOutcome {
    /// 非零退出（失败/超时）且 QEMU 有输出时，把 stderr 尾部打到 stderr 侧；
    /// 成功保持安静——全文在 qemu-stderr.log，triage 会给出路径。
    pub fn report_tail_on_failure(&self, code: i32) {
        if code == 0 || self.qemu_lines == 0 {
            return;
        }
        let omitted = self.qemu_lines - self.qemu_tail.len();
        let head = if omitted > 0 {
            format!("（前 {omitted} 行略）")
        } else {
            String::new()
        };
        eprintln!(
            "---------- QEMU stderr 尾部（{} 行）{head} ----------",
            self.qemu_tail.len()
        );
        for line in &self.qemu_tail {
            eprintln!("{line}");
        }
    }
}

/// 把已 spawn 的子进程两条流分开泵送（追加落盘；仅 stdout 回显终端）。
/// 由 launcher::QemuInvocation::spawn(piped=true) 提供子进程。
pub fn pump_child(
    child: &mut Child,
    out_log: &Path,
    err_log: &Path,
) -> std::io::Result<PumpOutcome> {
    let out = child.stdout.take().expect("stdout piped");
    let err = child.stderr.take().expect("stderr piped");

    fn pump_out(pipe: impl Read, log: &Path) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        let mut reader = std::io::BufReader::new(pipe);
        let stdout = std::io::stdout();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            {
                let mut lock = stdout.lock();
                lock.write_all(&buf)?;
                lock.flush()?;
            }
            file.write_all(&buf)?;
        }
        Ok(())
    }

    // stderr：只落盘；尾部摘录进有界共享缓冲，供 PumpOutcome 带回。
    fn pump_err(
        pipe: impl Read,
        log: &Path,
        collected: &std::sync::Mutex<(usize, Vec<String>)>,
    ) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        let mut reader = std::io::BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf)?;
            let mut state = collected.lock().unwrap();
            state.0 += 1;
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim_end_matches(['\n', '\r']);
            if state.1.len() >= QEMU_TAIL_LINES {
                state.1.remove(0);
            }
            state.1.push(line.to_string());
        }
        Ok(())
    }

    let out_log = out_log.to_path_buf();
    let err_log = err_log.to_path_buf();
    let collected = std::sync::Arc::new(std::sync::Mutex::new((0usize, Vec::new())));
    let t_out = std::thread::spawn(move || pump_out(out, &out_log));
    let t_err = {
        let collected = std::sync::Arc::clone(&collected);
        std::thread::spawn(move || pump_err(err, &err_log, &collected))
    };

    let status = child.wait()?;
    let _ = t_out.join();
    let _ = t_err.join();
    let (qemu_lines, qemu_tail) = collected.lock().unwrap().clone();
    Ok(PumpOutcome {
        status,
        qemu_lines,
        qemu_tail,
    })
}

// ---------------------------------------------------------------- 工件落盘/回读

fn read_serial_audit(serial: &Path) -> Audit {
    let text = std::fs::read_to_string(serial).unwrap_or_default();
    judge::parse(&text)
}

/// 运行结束后写 events.jsonl + verdict.json，返回 verdict 字符串。
pub fn finalize_run(run: &RunDir, meta: &RunMeta) -> anyhow::Result<String> {
    let audit = read_serial_audit(&run.path.join("serial.log"));
    let verdict = meta.verdict_of(&audit);

    let mut events = audit.events.clone();
    events.push(Event {
        line_no: None,
        kind: EventKind::RunEnd,
        text: String::new(),
        seq: events.len(),
        extra: serde_json::json!({
            "exit_code": meta.exit_code,
            "duration_ms": meta.duration_ms,
            "verdict": verdict.as_str(),
        }),
    });
    let mut ev_body = String::new();
    for e in &events {
        ev_body.push_str(&serde_json::to_string(e).context("events.jsonl 序列化失败")?);
        ev_body.push('\n');
    }
    std::fs::write(run.path.join("events.jsonl"), ev_body)?;

    let mut report = VerdictReport::build(&audit, meta);
    report.artifacts = Some(Artifacts {
        serial: run.path.join("serial.log").display().to_string(),
        qemu_stderr: run.path.join("qemu-stderr.log").display().to_string(),
        build_log: run.path.join("build.log").display().to_string(),
        events: run.path.join("events.jsonl").display().to_string(),
    });
    std::fs::write(
        run.path.join("verdict.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(verdict.as_str().to_string())
}

/// verdict.json 缺失（Ctrl-C/SIGKILL 中断）时的兜底：现场解析 serial.log。
pub fn load_verdict_or_parse(run_dir: &Path) -> anyhow::Result<VerdictReport> {
    let vpath = run_dir.join("verdict.json");
    if let Ok(text) = std::fs::read_to_string(&vpath) {
        return serde_json::from_str(&text).context("verdict.json 解析失败");
    }
    let audit = read_serial_audit(&run_dir.join("serial.log"));
    let id = run_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut report = VerdictReport::build(
        &audit,
        &RunMeta {
            run_id: id,
            arch: run_dir
                .file_name()
                .and_then(|n| {
                    n.to_string_lossy()
                        .rsplit_once('-')
                        .map(|(_, a)| a.to_string())
                })
                .unwrap_or_default(),
            exit_code: None,
            duration_ms: 0,
            timeout_s: String::new(),
            kernel: None,
            qemu_version: None,
            topo: serde_json::Value::Null,
            build_failed: false,
        },
    );
    report.verdict = judge::Verdict::Unknown;
    report.verdict_source =
        Some("serial.log fallback（运行未正常收尾，verdict.json 缺失）".to_string());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// 起一个真实子进程分离泵送：stdout 进 out_log 并回显，stderr 只进
    /// err_log，PumpOutcome 带回行数与尾部摘录。
    fn pump_shell(script: &str, out_log: &Path, err_log: &Path) -> PumpOutcome {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        pump_child(&mut child, out_log, err_log).expect("pump_child")
    }

    #[test]
    fn pump_routes_streams_to_their_own_logs() {
        let dir = std::env::temp_dir().join(format!("virtuoso-pump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (out_log, err_log) = (dir.join("serial.log"), dir.join("qemu-stderr.log"));

        let outcome = pump_shell(
            "echo guest-boot; echo qemu-noise >&2; echo guest-test",
            &out_log,
            &err_log,
        );
        assert_eq!(outcome.status.code(), Some(0));
        assert_eq!(outcome.qemu_lines, 1);
        assert_eq!(outcome.qemu_tail, ["qemu-noise"]);
        let serial = std::fs::read_to_string(&out_log).unwrap();
        assert!(serial.contains("guest-boot") && serial.contains("guest-test"));
        let err = std::fs::read_to_string(&err_log).unwrap();
        assert_eq!(err, "qemu-noise\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn qemu_tail_is_bounded_and_keeps_the_last_lines() {
        let dir = std::env::temp_dir().join(format!("virtuoso-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (out_log, err_log) = (dir.join("serial.log"), dir.join("qemu-stderr.log"));

        let outcome = pump_shell(
            "i=1; while [ $i -le 60 ]; do echo \"noise-$i\" >&2; i=$((i+1)); done",
            &out_log,
            &err_log,
        );
        assert_eq!(outcome.qemu_lines, 60);
        assert_eq!(outcome.qemu_tail.len(), QEMU_TAIL_LINES);
        assert_eq!(
            outcome.qemu_tail[0],
            format!("noise-{}", 60 - QEMU_TAIL_LINES + 1)
        );
        assert_eq!(
            outcome.qemu_tail.last().map(String::as_str),
            Some("noise-60")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
