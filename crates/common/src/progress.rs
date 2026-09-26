//! 进度输出：同时打印到终端，可选追加到运行工件（如 build.log）。
//! builder（构建流水线）与 forge（kernel 命令组）共用同一形态。

/// 进度输出通道：stdout 恒打，可选同步写日志文件。
pub struct Progress {
    log: Option<std::fs::File>,
}

impl Progress {
    /// 仅终端形态。
    pub fn stdout() -> Self {
        Self { log: None }
    }

    /// 终端 + 追加日志形态（运行工件 build.log）。
    pub fn with_log(path: &std::path::Path) -> std::io::Result<Self> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { log: Some(log) })
    }

    pub fn line(&mut self, msg: &str) {
        println!("{msg}");
        if let Some(f) = self.log.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
    }
}
