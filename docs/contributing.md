# 贡献约定

## 验证循环

改 Rust 源码必须先跑 `cargo clippy --all-targets -- -D warnings`（CI 同款
门禁，本地不过别提交）；再 `virtuoso doctor → virtuoso test`。判定以
test 收尾的 verdict 行为准，机读唯一面 = run 目录下的 `verdict.json`；
`verdict: passed` 才算通过。

## 约定

- VM 内 shell 代码保持 POSIX 兼容（`infra/init` / `infra/init-initramfs`
  跑在 BusyBox `sh`，不是 bash）；改完先 `busybox sh -n` 语法校验。
- harness 暴露新行为 → 配一个对应测试用例。
- 用户可见行为变化 → 更新本文档站（`docs/`）与
  `devkit/skills/kernel-dev/SKILL.md`。
- 冻结项不许动：标记协议 v1、test 退出码语义、argv 冻结基线——
  详见[冻结契约](concepts/contracts.md)与 AGENTS.md「冻结的不变量」。
- 文档只改 `docs/`（文档唯一事实来源，别处引用不复制内容）；push main 自动
  发布 gh-pages。

## CI/CD

`.github/workflows/` 四条流水线：

| 工作流 | 内容 |
|---|---|
| `virtuoso-ci.yml` | build / clippy（`-D warnings`）/ unit test；E2E 走自建 runner（openEuler 宿主 + KVM），手动 `workflow_dispatch` 触发，失败时上传 `target/runs/` 整体工件 |
| `busybox-release.yml` | 三架构静态 BusyBox 预编译发布 GitHub Release；构建环境钉死 `ubuntu:22.04` 容器（busybox 1.36.1 的 tc applet 无法在内核头文件 ≥ 6.8 下编译，交叉 gcc 行为随发行版漂移）；交叉编译必须显式安装 `libc6-dev-<arch>-cross`（`--no-install-recommends` 会漏装） |
| `kernel-builder.yml` | 发布容器化内核构建器镜像到 ghcr（`ghcr.io/yxalix/virtuoso-kernel`，linux/arm64 + linux/amd64 双基座）；devkit/docker/ 变更推 main 或手动 dispatch 触发；镜像纯为提速，拉不动时 forge 回落本地构建 |
| `docs.yml` | mdBook 构建 docs/ → GitHub Pages（gh-pages） |
