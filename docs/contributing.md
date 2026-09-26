# 贡献约定

## 代码约定

- VM 内 shell 代码保持 POSIX 兼容（`init` / `init-initramfs` 跑在 BusyBox
  `sh`，不是 bash）；改完先 `busybox sh -n` 语法校验。
- 新增前置条件 → 同步 `src/builder/verify.rs`（检查引擎单文件：类型与渲染、
  `CheckInput`/`run_checks` 编排、逐项检查函数分节；doctor 的分组呈现自动跟随，
  见 `src/cli/doctor.rs`）。
- harness 暴露新行为 → 配一个对应测试用例。
- 用户可见行为变化 → 更新本文档站（`docs/`）与 `devkit/skills/kernel-dev/SKILL.md`。
- 冻结项不许动：标记协议 v1、test 退出码语义、`QemuInvocation::argv` 基线
  （详见[冻结契约](architecture/contracts.md) 与 AGENTS.md「冻结的不变量」）。
- 文档只改 `docs/`（文档唯一事实来源，别处引用不复制内容）；push main 自动
  发布 gh-pages。

## CI/CD

`.github/workflows/` 三条流水线：

| 工作流 | 内容 |
|---|---|
| `virtuoso-ci.yml` | build / clippy（`-D warnings`）/ unit test；E2E 走自建 runner（openEuler 宿主 + KVM），手动 `workflow_dispatch` 触发，失败时上传 `target/runs/` 整体工件 |
| `busybox-release.yml` | 三架构静态 BusyBox 预编译发布 GitHub Release；构建环境钉死 `ubuntu:22.04` 容器（busybox 1.36.1 的 tc applet 无法在内核头文件 ≥ 6.8 下编译，交叉 gcc 行为随发行版漂移）；交叉编译必须显式安装 `libc6-dev-<arch>-cross`（`--no-install-recommends` 会漏装） |
| `docs.yml` | mdBook 构建 docs/ → GitHub Pages（gh-pages） |
