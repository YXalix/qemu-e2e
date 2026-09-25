//! devcontainer 渲染：current 切换 → `.devcontainer/devcontainer.json`（git 忽略）。
//!
//! devkit/docker/devcontainer.json 是静态模板（workspaceMount 钉死缺省卷
//! virtuoso-kernel），对 `--as` 命名的多卷无感知——named volume 的多内核
//! 并存与固定卷名矛盾。这里在 `state::write`（clone/use 写 current 的唯一
//! 入口）同步渲染一份活动卷专属的 devcontainer.json：workspaceMount 指向
//! current 卷，image = 钉死工具链镜像（复用 `KERNEL_TOOLCHAIN_IMAGE` 覆盖；
//! 首进免本地 build Dockerfile.kernel）。
//!
//! 落点必须是 repo 根的 `.devcontainer/`：VS Code 的自动发现契约只扫打开
//! 工作区下的 `.devcontainer/`（或根级 devcontainer.json）——放 `.virtuoso/`
//! 就只剩手动选隐藏目录一条路（macOS 文件夹选择器还默认不显示 dotfile）。
//! 打开 repo 根 →「Reopen in Container」即进 current 卷的 /ksrc。

use std::path::{Path, PathBuf};

use crate::state::Current;
use crate::toolchain;

/// 模板（JSONC，devcontainer.json 规范允许注释）：{VOLUME} / {IMAGE} 由
/// render 替换；customizations 与静态模板保持同源。
const TEMPLATE: &str = r#"// 由 `virtuoso kernel use/clone` 自动渲染 —— 勿手编，切卷重跑 `virtuoso kernel use`。
// 打开 repo 根：VS Code →「Reopen in Container」（自动发现本文件）即进 /ksrc。
// workspace = /ksrc（current 活动 named volume）；镜像 = 钉死工具链
// （env KERNEL_TOOLCHAIN_IMAGE 渲染时已代入）。
{
  "name": "virtuoso-kernel",
  "image": "{IMAGE}",
  "workspaceMount": "src={VOLUME},dst=/ksrc,type=volume",
  "workspaceFolder": "/ksrc",
  "remoteUser": "root",
  "customizations": {
    "vscode": {
      "extensions": [
        "llvm-vs-code-extensions.vscode-clangd",
        "ms-azuretools.vscode-docker"
      ],
      "settings": {
        // clangd 在容器内跑：吃 /ksrc/.clangd + compile_commands.json
        "clangd.arguments": [
          "--background-index",
          "--clang-tidy=false",
          "--header-insertion=never",
          "--query-driver=/usr/bin/*"
        ],
        // 内核 C 风格：8 空格缩进
        "editor.tabSize": 8,
        "[c]": { "editor.insertSpaces": false },
        "files.exclude": {
          "**/*.o": true,
          "**/*.cmd": true,
          "**/*.ko": true,
          "**/.*.cmd": true
        }
      }
    }
  }
}
"#;

/// 渲染产物路径（repo 根 `.devcontainer/devcontainer.json`，git 忽略；落点
/// 由 VS Code 自动发现契约决定，见模块文档）。
pub fn path(project_root: &Path) -> PathBuf {
    project_root.join(".devcontainer").join("devcontainer.json")
}

/// 填模板（volume + image → JSONC 文本）。与写盘分离，测试直接消费。
pub fn render_template(volume: &str, image: &str) -> String {
    TEMPLATE.replace("{VOLUME}", volume).replace("{IMAGE}", image)
}

/// 按 current 渲染并写盘（镜像解析 env KERNEL_TOOLCHAIN_IMAGE > ghcr 发布镜像，
/// 与 kernel 命令组构建容器同源）。返回产物路径。旧落点 `.virtuoso/` 的产物
/// 一并清除（自动发现契约扫不到它，留着只会误导）。
pub fn render(project_root: &Path, current: &Current) -> anyhow::Result<PathBuf> {
    let body = render_template(&current.volume, &toolchain::resolve_image());
    let p = path(project_root);
    std::fs::create_dir_all(p.parent().expect("渲染路径必有父目录 .devcontainer"))?;
    std::fs::write(&p, body)?;
    let legacy = project_root.join(".virtuoso").join("devcontainer.json");
    if legacy.is_file() {
        let _ = std::fs::remove_file(&legacy);
    }
    Ok(p)
}

/// 剥 JSONC 注释行（模板头 + 内嵌说明），供测试以 serde_json 校验结构。
#[cfg(test)]
fn json_body(jsonc: &str) -> String {
    jsonc
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_targets_current_volume_and_image() {
        let out = render_template("ksrc-oe66", "ghcr.io/yxalix/virtuoso-kernel:latest");
        let v: serde_json::Value = serde_json::from_str(&json_body(&out)).expect("渲染产物须为合法 JSON");
        assert_eq!(v["workspaceMount"], "src=ksrc-oe66,dst=/ksrc,type=volume");
        assert_eq!(v["workspaceFolder"], "/ksrc");
        assert_eq!(v["image"], "ghcr.io/yxalix/virtuoso-kernel:latest");
        // 加速首进：image 直用钉死工具链，不再本地 build Dockerfile.kernel
        assert!(v.get("build").is_none());
        assert_eq!(v["remoteUser"], "root");
        assert!(v["customizations"]["vscode"]["extensions"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn render_template_is_idempotent() {
        let a = render_template("ksrc-mainline", "img:1");
        let b = render_template("ksrc-mainline", "img:1");
        assert_eq!(a, b);
        assert!(!a.contains("{VOLUME}") && !a.contains("{IMAGE}"));
    }
}
