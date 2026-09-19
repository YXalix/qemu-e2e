//! coda — 终曲：RAII 资源治理。
//! Phase 2 提供 ProcessGroupGuard（取代 Makefile 的 PID 文件 + kill hack）、
//! SafeMountGuard（loop 镜像挂载）与 panic/信号 hook，保证任何退出路径清场。
//! Phase 1 的进程组收割逻辑暂由 xtask/src/tasks.rs 承担。
