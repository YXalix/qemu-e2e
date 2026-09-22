//! 文档命令：mdBook 构建 / 本地预览。
//!
//! `docs/` 是文档唯一事实来源（`book.toml` 指向它，`create-missing = false`），
//! 产物落 `target/book`（数据面，git 忽略）；线上发布由
//! `.github/workflows/docs.yml` 推到 gh-pages。本模块只做进程接线，
//! 不复制任何文档内容。

use std::process::Command;

use super::code_of;

/// `cargo xtask docs`：构建文档；`--serve` 起本地预览服务
/// （默认 http://localhost:3000，改文件实时刷新）。
pub fn run_docs(serve: bool, open: bool) -> anyhow::Result<i32> {
    let mut cmd = Command::new("mdbook");
    cmd.arg(if serve { "serve" } else { "build" });
    if open {
        cmd.arg("--open");
    }
    match cmd.status() {
        Ok(status) => Ok(code_of(status)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(anyhow::anyhow!(
            "未找到 mdbook —— 先安装：cargo install mdbook --locked"
        )),
        Err(e) => Err(anyhow::Error::new(e).context("运行 mdbook 失败")),
    }
}
