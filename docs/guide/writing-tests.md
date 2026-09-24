# 编写测试用例

`init` 自动发现 rootfs `/tests/` 下的所有二进制，**新增用例零接线**。
两条硬规则：

- 测试必须**静态链接**（VM 内无动态加载器；musl 目标 crt-static 是
  rustc 缺省，workspace 配方已冻结，写用例不用管）；
- **禁止用 `|| true` 掩盖失败**。

用例统一走一个 cargo workspace（`infra/testcases/`）：**testfw（std
Rust）是框架与入口，C 测试体经用例 crate 的 build.rs（cc crate）编入同一
静态二进制**（C over Rust）。协议 v1 对 Rust/C 断言一视同仁——同一串口
文本、同一套计数器（C 宏经 FFI 落回 testfw，冻结文本单一来源）。

## 新建用例（复制 test-example）

1. 复制 `infra/testcases/test-example/` 为 `test-<name>/`，并把
   workspace `Cargo.toml` 的 `members` 加上一行。crate 名即二进制名，
   会出现在串口 `--- Running: test-<name> ---`。

2. **Rust 测试**：`src/main.rs` 的 `TESTS` 注册 `("名称", fn() -> bool)`，
   断言用 `testfw::check!`（或 `pass!/fail!/skip!` 手动组合）：

```rust
fn my_feature() -> bool {
    // 通过 syscall / /proc / /sys / /dev 驱动被测内核（std 可用）
    testfw::check!(!std::fs::read_to_string("/proc/uptime").is_err(), "…")
}
```

3. **C 测试**：`c/` 下放 `.c` 文件（build.rs 自动收集编译），断言用
   `testfw.h` 的宏（用法与旧 `test_common.h` 一致），在 `run_c_tests` 里
   逐个调用（入口名与 `main.rs` 的 `extern "C"` 声明一致）：

```c
#include "testfw.h"

static void test_my_feature(void)
{
    if (/* 期望条件 */)
        PASS("the thing happened");
    else
        FAIL("expected X, got Y");
}

void run_c_tests(void)
{
    test_my_feature();
}
```

4. 重建并运行：

```bash
virtuoso build && virtuoso test --timeout 30
```

> IDE：仓库自带 `.vscode/settings.json`（rust-analyzer 索引本 workspace，
> 并覆盖用户级 clangd 参数——`--compile-commands-dir` 会把 CDB 查找钉死
> 在单一路径、关掉沿祖先目录的自动搜索，必须避免）；C 侧 clangd 吃各用例
> crate 的 build.rs 生成的 `<crate>/compile_commands.json`（跑一次
> `cargo check` 即生成，交叉构建会被 `virtuoso build` 覆写为 zig 条目，
> 再跑一次 `cargo check` 恢复 host 版）。

## 断言输出规则

- 测试二进制只输出 `[PASS]` / `[FAIL]` / `[SKIP]` / `[INFO]` 断言行
  （一律走宏，不要直接 `printf`/`println!` 协议前缀）；
- `--- Running:` / `PASSED:` / `FAILED:` / `Test Results:` /
  `TEST_COMPLETE:` 是 **init 的汇编层标记**，测试程序禁止输出；
- 退出码由 testfw 汇总（任何 FAIL > 0 → 1），C 段计数经 FFI 同源，
  用例代码不需要自己管理计数器与退出码。

协议文本冻结，逐条语义见[冻结契约](../architecture/contracts.md)。

## 需要内核模块的测试

模块清单由组件 `require` 生成，不手写 conf——见
[组件机制](../components/overview.md)。迭代中的模块优先 `=m` 而非 `=y`，
免内核重建。
