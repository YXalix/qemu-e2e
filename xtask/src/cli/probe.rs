//! probe — AI 交互通道：经 virtio-serial agent（tools/virtuoso-agent）下发
//! 命令批，结构化事件流回吐。构建 → agent 串口启动 → ping 握手 → 逐条
//! 下发 → 收割。运行工件：run 目录的 serial.log + agent-events.jsonl。
//!
//! 退出码沿用 judge::exit 契约：0 = 全部命令 exit 0；124 = 看门狗超时；
//! 其余 = 失败（协议中断/命令非零退出）。

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time::{Duration, Instant};

use anyhow::Context;
use serde_json::{json, Value};

/// 单次协议读的底层超时（轮询粒度，决定对看门狗 KILL 的响应速度）。
const IO_TICK: Duration = Duration::from_millis(500);

pub fn run_probe(
    cli_arch: Option<&str>,
    cli_timeout: Option<u64>,
    cmds: &[String],
    cmd_file: Option<&Path>,
    json: bool,
) -> anyhow::Result<i32> {
    let cfg = crate::config::Config::load()?;
    let arch = super::resolve_arch(&cfg, cli_arch)?;
    let topo = super::resolve_topology(&cfg)?;
    let timeout_secs = match cli_timeout {
        Some(0) => anyhow::bail!("probe timeout must be > 0"),
        Some(t) => t,
        None => 300,
    };

    // ---- 构建（probe 恒开 agent 通道：强制把 virtio_console 并入 runtime
    // 模块清单，不依赖 [components.agent] 开关；agent 随 tools.img 装入
    // /bin，musl target 缺失时 WARN 跳过，后续握手超时会给出明确指引）----
    super::build::build_pair_for(&cfg, None, &["virtio_console".to_string()])?;

    let commands = collect_commands(cmds, cmd_file)?;
    anyhow::ensure!(
        !commands.is_empty(),
        "no commands: use --cmd <shell> (repeatable) or --cmd-file <file>"
    );

    let run = crate::runs::create_run_dir(&cfg.project_root, arch.name())?;
    println!("[RUN] artifacts: {}", run.path.display());

    let sock_path = run.path.join("agent.sock");
    let _ = std::fs::remove_file(&sock_path); // QEMU 不清理已存在的 socket 路径

    // ---- 启动（agent 串口 + piped 串口捕获；TCG 缺省，KVM 走 shell 模式）----
    let kernel = super::kernel_image_path(&cfg, arch)?;
    let inv = launcher::QemuInvocation::new(
        arch,
        &kernel,
        cfg.artifacts_dir.join("initrd.img"),
        cfg.artifacts_dir.join("rootfs.img"),
    )
    .accel(launcher::Accel::Tcg)
    .pmem(super::pmem_opt(&cfg, arch, &topo)?)
    .topo(topo)
    .qemu_override(cfg.qemu_override().as_deref())
    .virtio_disks(super::tools_disk_opt(&cfg))
    .agent_serial(&sock_path)
    .extra_opts(&cfg.qemu_extra());
    println!(
        "[LAUNCH] {}",
        inv.command_line().map_err(anyhow::Error::msg)?
    );
    let (mut child, mut sup) = inv.spawn_supervised(true)?;

    // 串口泵放工作线程（阻塞读），主线程跑 agent 协议；总超时到点看门狗
    // KILL 进程组 → 串口与 socket 双双 EOF，两条线程自然汇合。
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog =
        guardian::registry::spawn_watchdog(sup.pgid(), timeout_secs, Arc::clone(&timed_out));
    let serial_log = run.path.join("serial.log");
    let stderr_log = run.path.join("qemu-stderr.log");
    let pump = std::thread::spawn(move || {
        let status = crate::runs::pump_child(&mut child, &serial_log, &stderr_log);
        (child, status)
    });

    let started = Instant::now();
    let deadline = started + Duration::from_secs(timeout_secs);
    let outcome = session(&sock_path, &commands, json, &run.path, deadline);

    // 无论成败：收 VM、汇合线程、结清看门狗（先取快照再置退出标志，
    // 看门狗轮询该标志退出 —— 顺序与 test_once 一致，避免误标超时）
    let timed_out_now = timed_out.load(Ordering::SeqCst);
    sup.kill_now();
    timed_out.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    let pump = pump
        .join()
        .map_err(|_| anyhow::anyhow!("serial pump thread panicked"));
    if let Ok((mut child, _)) = pump {
        let _ = child.wait();
    }
    sup.finish();

    let exit_code = match &outcome {
        Ok(all_ok) if !timed_out_now => {
            if *all_ok {
                0
            } else {
                1
            }
        }
        _ if timed_out_now => judge::exit::EXIT_TIMEOUT,
        _ => 1,
    };
    match &outcome {
        Ok(true) => println!("[RUN] probe: all commands passed"),
        Ok(false) => println!("[RUN] probe: some commands failed"),
        Err(e) => eprintln!("ERROR: probe session: {e:#}"),
    }
    println!(
        "[RUN] probe finished ({:.1}s, exit {exit_code}) — 事件: {}/agent-events.jsonl",
        started.elapsed().as_secs_f32(),
        run.path.display()
    );
    Ok(exit_code)
}

/// 协议会话：连接 → hello/ping 握手 → 逐条命令。返回 Ok(true) = 全部通过。
fn session(
    sock_path: &Path,
    commands: &[String],
    json: bool,
    run_dir: &Path,
    deadline: Instant,
) -> anyhow::Result<bool> {
    let sock = connect(sock_path, deadline).context("agent socket connect")?;
    sock.set_read_timeout(Some(IO_TICK)).ok();

    let events_path = run_dir.join("agent-events.jsonl");
    let mut events = std::fs::File::create(&events_path)
        .with_context(|| format!("create {}", events_path.display()))?;
    let mut wire = LineSock::new(sock);

    let hello = wire.next_line(deadline)?.context(
        "EOF before agent hello (virtio_console 模块未加载或 agent 未启动？查 serial.log)",
    )?;
    log_event(&mut events, &hello, json)?;
    if !hello.contains("\"hello\"") {
        anyhow::bail!("unexpected first line from agent: {hello}");
    }
    // ping 握手：确认协议往返可用（modules.conf 加载完成、agent 就绪）
    let pong = exchange(&mut wire, &mut events, json, 0, "ping", |v| {
        v.get("type").and_then(Value::as_str) == Some("pong")
    })?;
    anyhow::ensure!(
        pong.lines.iter().any(|l| l.contains("\"pong\"")),
        "ping handshake failed"
    );

    let mut all_ok = true;
    for (idx, cmd) in commands.iter().enumerate() {
        let id = (idx + 1) as u64;
        if !json {
            println!("[{id}] $ {cmd}");
        }
        let reply = exchange(&mut wire, &mut events, json, id, cmd, |v| {
            v.get("type").and_then(Value::as_str) == Some("exit")
        })?;
        let exit_code = reply.exit_code;
        let had_error = reply.had_error;
        match exit_code {
            Some(0) => {}
            Some(c) => {
                all_ok = false;
                if !json {
                    println!("[{id}] exit {c}");
                }
            }
            None => {
                all_ok = false;
                if !json {
                    println!("[{id}] no exit event (protocol interrupted)");
                }
            }
        }
        if had_error {
            all_ok = false;
        }
    }
    Ok(all_ok)
}

struct Reply {
    /// 收到的全部事件行（JSON 字符串）
    lines: Vec<String>,
    /// exit 事件的 code（未收到 = None）
    exit_code: Option<i64>,
    /// 是否出现 error 事件
    had_error: bool,
}

/// 发送一条请求并按谓词收集事件（命令等 exit；ping 等 pong —— ping 没有
/// exit 事件，不能与命令共用「等 exit」的终止条件）。
fn exchange(
    wire: &mut LineSock,
    events: &mut std::fs::File,
    json: bool,
    id: u64,
    cmd: &str,
    done: impl Fn(&Value) -> bool,
) -> anyhow::Result<Reply> {
    let req = json!({"id": id, "cmd": cmd}).to_string();
    wire.send_line(&req)?;
    let mut reply = Reply {
        lines: Vec::new(),
        exit_code: None,
        had_error: false,
    };
    loop {
        let line = wire
            .next_line(Instant::now() + Duration::from_secs(600))?
            .context(
                "EOF while waiting for agent reply (VM died? watchdog fired? 查 serial.log)",
            )?;
        log_event(events, &line, json)?;
        let ev: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if done(&ev) {
            if let Some("exit") = ev.get("type").and_then(Value::as_str) {
                reply.exit_code = ev.get("code").and_then(Value::as_i64);
            }
            reply.lines.push(line);
            return Ok(reply);
        }
        match ev.get("type").and_then(Value::as_str) {
            Some("error") => {
                reply.had_error = true;
                reply.lines.push(line);
            }
            _ => reply.lines.push(line),
        }
    }
}

/// 事件落盘 + 终端呈现（json 模式原样透传，human 模式摘要行）。
fn log_event(events: &mut std::fs::File, line: &str, json: bool) -> std::io::Result<()> {
    events.write_all(line.as_bytes())?;
    events.write_all(b"\n")?;
    events.flush()?;
    if json {
        println!("{line}");
        return Ok(());
    }
    let Ok(ev) = serde_json::from_str::<Value>(line) else {
        println!("raw: {line}");
        return Ok(());
    };
    let id = ev.get("id").and_then(Value::as_u64);
    match (
        ev.get("type").and_then(Value::as_str),
        ev.get("line").and_then(Value::as_str),
    ) {
        (Some(t @ ("out" | "err")), Some(l)) => println!("{}{t}: {l}", tag(id)),
        // exit 事件由 session 统一汇报（含"未收到 exit"的失败形态），此处静默
        (Some("exit"), _) => {}
        (Some("hello"), _) => println!("{}agent hello", tag(id)),
        (Some("pong"), _) => println!("{}pong", tag(id)),
        (Some("error"), _) => println!(
            "{}error: {}",
            tag(id),
            ev.get("reason").and_then(Value::as_str).unwrap_or("?")
        ),
        _ => println!("raw: {line}"),
    }
    Ok(())
}

fn tag(id: Option<u64>) -> String {
    id.map(|i| format!("[{i}] "))
        .unwrap_or_else(|| "[agent] ".into())
}

/// 连接 agent socket：QEMU 立即监听，guest agent 要等引导完成才会说话，
/// 但 chardev 连接本身在启动后即成功 —— 连接成功 ≠ agent 就绪，就绪由
/// hello 行确认。
fn connect(sock_path: &Path, deadline: Instant) -> std::io::Result<UnixStream> {
    loop {
        match UnixStream::connect(sock_path) {
            Ok(s) => return Ok(s),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(IO_TICK);
            }
            Err(e) => return Err(e),
        }
    }
}

fn collect_commands(cmds: &[String], cmd_file: Option<&Path>) -> anyhow::Result<Vec<String>> {
    let mut all: Vec<String> = cmds.to_vec();
    if let Some(f) = cmd_file {
        let text = std::fs::read_to_string(f).with_context(|| format!("read {}", f.display()))?;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            all.push(line.to_string());
        }
    }
    Ok(all)
}

/// unix socket 行读取器：自管缓冲 + 轮询超时（BufReader 在 TimedOut 时可能
/// 吞掉已缓冲的部分行，这里逐字节搬移避免丢失）。
struct LineSock {
    sock: UnixStream,
    buf: Vec<u8>,
    pos: usize,
    fill: usize,
}

impl LineSock {
    fn new(sock: UnixStream) -> Self {
        Self {
            sock,
            buf: vec![0; 8192],
            pos: 0,
            fill: 0,
        }
    }

    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        self.sock.write_all(&bytes)
    }

    /// 读一行（不含 \n）。deadline 内无完整行返回 Err；EOF 且无残留返回 Ok(None)。
    fn next_line(&mut self, deadline: Instant) -> anyhow::Result<Option<String>> {
        let mut line = Vec::new();
        loop {
            if self.pos < self.fill {
                if let Some(nl) = self.buf[self.pos..self.fill]
                    .iter()
                    .position(|&b| b == b'\n')
                {
                    line.extend_from_slice(&self.buf[self.pos..self.pos + nl]);
                    self.pos += nl + 1;
                    return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
                }
                line.extend_from_slice(&self.buf[self.pos..self.fill]);
                self.pos = self.fill;
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timeout waiting for agent line ({} bytes buffered)",
                    line.len()
                );
            }
            match self.sock.read(&mut self.buf) {
                Ok(0) => {
                    return if line.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(String::from_utf8_lossy(&line).into_owned()))
                    };
                }
                Ok(n) => {
                    self.buf.copy_within(0..n, 0);
                    self.pos = 0;
                    self.fill = n;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_file_skips_comments_and_blank_lines() {
        let dir = std::env::temp_dir().join(format!("probe-cmds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("cmds.txt");
        std::fs::write(&f, "# comment\n\nuname -a\n  dmesg | tail -2  \n").unwrap();
        let cmds = collect_commands(&["echo hi".to_string()], Some(&f)).unwrap();
        assert_eq!(cmds, ["echo hi", "uname -a", "dmesg | tail -2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn connect_refuses_before_socket_exists() {
        let missing =
            std::env::temp_dir().join(format!("probe-absent-{}.sock", std::process::id()));
        let started = Instant::now();
        let r = connect(&missing, started + Duration::from_millis(1200));
        assert!(r.is_err());
        assert!(started.elapsed() >= Duration::from_millis(1100));
    }
}
