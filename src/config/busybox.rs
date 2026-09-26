//! BusyBox 供给配置（[busybox] 段 + BUSYBOX_* 环境变量）。

use builder::busybox;

use super::{env_bool, scalar, Config};

impl Config {
    /// BusyBox 供给配置（BUSYBOX_* 变量优先，回落 toml [busybox] 段）。
    pub fn busybox_supply(&self) -> busybox::Supply {
        let toml = self.toml.as_ref().and_then(|t| t.busybox.as_ref());
        busybox::Supply {
            version: scalar(toml.and_then(|b| b.version.as_ref()), "BUSYBOX_VERSION"),
            release_repo: scalar(
                toml.and_then(|b| b.release_repo.as_ref()),
                "BUSYBOX_RELEASE_REPO",
            ),
            dl_url: scalar(toml.and_then(|b| b.dl_url.as_ref()), "BUSYBOX_DL_URL"),
            force_source_build: env_bool("BUSYBOX_SOURCE_BUILD")
                .unwrap_or_else(|| toml.and_then(|b| b.force_source_build).unwrap_or(false)),
        }
    }
}
