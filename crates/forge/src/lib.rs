//! forge — 内核锻造：容器化内核源码供给（named volume + 钉死工具链镜像）。
//!
//! 收编 `devkit/docker/kernel.sh`（薄壳退役，CI 发布的 Dockerfile.kernel
//! 保留在原位）：源码权威存 named volume（容器侧 ext4：大小写敏感 + 构建
//! 性能），宿主经平台视图直接读写——macOS = OrbStack 视图、Linux = volume
//! 本体。`virtuoso kernel` 命令组投影到本 crate：clone/defconfig/build/
//! shell + list/use（卷管理与 current 切换，活动卷状态落 repo 根
//! `.virtuoso/kernel-current.json`）。源码编辑走 VS Code devcontainer
//! （容器内 clangd 吃 build 产出的 /ksrc 原始形态 compile_commands.json）。
//!
//! 测试主循环在宿主原生跑：KERNEL_PATH 指 `virtuoso kernel path` 的输出
//! + `virtuoso doctor / build / test`。

pub mod clone;
pub mod state;
pub mod toolchain;
pub mod volume;

/// 缺省卷名（每卷自含源码 + .config + 增量产物，切回免重编）。
pub const DEFAULT_VOLUME: &str = "virtuoso-kernel";
/// 钉死工具链镜像（kernel-builder.yml 发布 ghcr；pull 失败回落本地构建
/// devkit/docker/Dockerfile.kernel）。
pub const DEFAULT_IMAGE: &str = "ghcr.io/yxalix/virtuoso-kernel:latest";
/// clone 缺省 ref。
pub const DEFAULT_REF: &str = "master";

/// 进度输出（stdout 形态；kernel 命令组无 build.log 需求，不走 builder::Progress
/// 以免 crate 依赖倒挂）。
pub struct Progress;

impl Progress {
    pub fn stdout() -> Self {
        Self
    }

    pub fn line(&mut self, msg: &str) {
        println!("{msg}");
    }
}
