//! virtuoso-agent — VM 内常驻 agent。
//!
//! 传输层：virtio-serial 端口（QEMU 侧 virtserialport name=virtuoso-agent，
//! 宿主侧 chardev unix socket）。协议：JSON 行（每行一个对象）。
//!
//! - 启动即发 `{"type":"hello","agent":"virtuoso-agent","proto":1}`；
//! - 请求 `{"id":N,"cmd":"<shell 命令>"}`：经 /bin/sh -c 执行，stdout/stderr
//!   逐行以 `{"id":N,"type":"out"|"err","line":"..."}` 回吐，结束时发
//!   `{"id":N,"type":"exit","code":C}`（信号死亡归一为 128+sig）；
//! - 保留命令 `ping` → `{"id":N,"type":"pong"}`（宿主握手用）；
//! - 解析失败的行回 `{"type":"error","reason":"..."}`（无 id）。
//!
//! 设备发现：优先现成节点，再扫 sysfs class 并按 dev 属性 mknod 兜底
//! （openEuler devtmpfs 留空 /dev 的实测怪癖）。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use std::os::unix::process::ExitStatusExt;

const PORT_NAME: &str = "virtuoso-agent";

fn main() {
    // std 运行时默认忽略 SIGPIPE；这里不做额外信号配置。
    let Some(port) = open_agent_port() else {
        // 无端口 = 未挂 virtio-serial 设备或驱动未加载：静默退出（init 守卫式启动）
        eprintln!("virtuoso-agent: no {PORT_NAME} virtio-serial port found");
        std::process::exit(1);
    };
    if send(
        &port,
        json!({"type": "hello", "agent": PORT_NAME, "proto": 1}),
    )
    .is_err()
    {
        return; // 端口立刻失效（宿主未连即断）：退出
    }
    let reader = match port.try_clone() {
        Ok(f) => BufReader::new(f),
        Err(_) => return,
    };
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if handle_request(&port, &line).is_err() {
            break; // 端口写失败：宿主已断开
        }
    }
}

/// 处理一条请求行；Err = 端口不可写。
fn handle_request(port: &File, line: &str) -> std::io::Result<()> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return send(
                port,
                json!({"type": "error", "reason": format!("bad request: {e}")}),
            );
        }
    };
    let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
    let Some(cmd) = req.get("cmd").and_then(Value::as_str) else {
        return send(
            port,
            json!({"id": id, "type": "error", "reason": "missing cmd"}),
        );
    };
    if cmd == "ping" {
        return send(port, json!({"id": id, "type": "pong"}));
    }
    run_command(port, id, cmd)
}

/// fork+exec /bin/sh -c 并逐行回吐输出。两个读端各一个线程，行事件经
/// channel 汇到主线程串行写端口（端口是单写者流，不能并发写）。
fn run_command(port: &File, id: u64, cmd: &str) -> std::io::Result<()> {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return send(
                port,
                json!({"id": id, "type": "error", "reason": format!("spawn /bin/sh: {e}")}),
            );
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<(&'static str, String)>();
    let mut handles = Vec::new();
    if let Some(out) = stdout {
        handles.push(relay(out, "out", tx.clone()));
    }
    if let Some(err) = stderr {
        handles.push(relay(err, "err", tx.clone()));
    }
    drop(tx);
    // rx 结束 = 两个 relay 线程都已 EOF（senders 全部 drop），无需哨兵事件
    for ev in rx {
        send(port, json!({"id": id, "type": ev.0, "line": ev.1}))?;
    }
    for h in handles {
        let _ = h.join();
    }
    let code = match child.wait() {
        Ok(st) => match (st.code(), st.signal()) {
            (Some(c), _) => c,
            (None, Some(s)) => 128 + s,
            (None, None) => 127,
        },
        Err(e) => {
            return send(
                port,
                json!({"id": id, "type": "error", "reason": format!("wait: {e}")}),
            );
        }
    };
    send(port, json!({"id": id, "type": "exit", "code": code}))
}

/// 逐行泵一个管道到 channel；EOF 时 sender 随线程结束被 drop（channel 关闭）。
fn relay(
    pipe: impl Read + Send + 'static,
    kind: &'static str,
    tx: std::sync::mpsc::Sender<(&'static str, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send((kind, line)).is_err() {
                return;
            }
        }
    })
}

/// 写一行 JSON 到端口（追加 \n，write_all 保证短写续写）。
fn send(port: &File, v: Value) -> std::io::Result<()> {
    let mut buf = v.to_string();
    buf.push('\n');
    (&*port).write_all(buf.as_bytes())
}

/// 定位名为 PORT_NAME 的 virtserialport 并以读写打开。
fn open_agent_port() -> Option<File> {
    // 1) 现成节点（devtmpfs 正常创建 / udev 命名目录）
    let direct = [
        format!("/dev/virtio-ports/{PORT_NAME}"),
        "/dev/vport0p0".to_string(),
        "/dev/vport0p1".to_string(),
    ];
    for path in &direct {
        if let Ok(f) = OpenOptions::new().read(true).write(true).open(path) {
            return Some(f);
        }
    }
    // 2) sysfs 扫描：/sys/class/virtio-ports/vportNpM/{name,dev}
    let class = "/sys/class/virtio-ports";
    let entries = std::fs::read_dir(class).ok()?;
    let mut names: Vec<_> = entries.flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for entry in names {
        let vport = entry.file_name().to_string_lossy().to_string();
        let name_path = format!("{class}/{vport}/name");
        let Ok(name) = std::fs::read_to_string(&name_path) else {
            continue;
        };
        if name.trim() != PORT_NAME {
            continue;
        }
        let node = format!("/dev/{vport}");
        if let Ok(f) = OpenOptions::new().read(true).write(true).open(&node) {
            return Some(f);
        }
        // 3) mknod 兜底：devtmpfs 未建节点时按 dev 属性自造（怪癖表同款）
        let dev_attr = std::fs::read_to_string(format!("{class}/{vport}/dev")).ok()?;
        let (maj, min) = dev_attr.trim().split_once(':')?;
        let (maj, min): (u32, u32) = (maj.parse().ok()?, min.parse().ok()?);
        let cpath = std::ffi::CString::new(node.as_bytes()).ok()?;
        // SAFETY: cpath 指向合法 C 字符串；mknodat(AT_FDCWD, ...) 为标准系统调用。
        let rc = unsafe {
            libc::mknodat(
                libc::AT_FDCWD,
                cpath.as_ptr(),
                libc::S_IFCHR | 0o600,
                libc::makedev(maj, min),
            )
        };
        if rc != 0 {
            eprintln!(
                "virtuoso-agent: mknod {node} failed: {}",
                std::io::Error::last_os_error()
            );
            continue;
        }
        return OpenOptions::new().read(true).write(true).open(&node).ok();
    }
    None
}
