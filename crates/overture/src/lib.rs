//! overture — 序曲：构建器。
//! Phase 2 接管 C 用例增量编译（`-static -O2 -Wall` + 交叉前缀）、
//! modules.conf 语义的 `.ko` 收集、BusyBox 缓存与 initrd 组装。
//! Phase 1 期间由 `infra/build-initrd.sh` 承担（xtask 包装调用）。
