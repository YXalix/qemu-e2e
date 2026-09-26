//! 用例构建脚本：通用实现全部在 coda-scaffold（CC 探测、c/*.c 编入、空桩、
//! per-crate compile_commands.json）。每个用例的 build.rs 都是这一行。

fn main() {
    coda_scaffold::run()
}
