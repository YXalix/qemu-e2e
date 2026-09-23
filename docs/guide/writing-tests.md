# 编写测试用例

`init` 自动发现 rootfs `/tests/` 下的所有二进制，**新增用例零接线**。
两条硬规则：

- 测试必须**静态链接**（`-static`，VM 内无动态加载器）；
- **禁止用 `|| true` 掩盖失败**。

C 与 no_std Rust 两条路径并存，协议 v1 对两者一视同仁（同一串口协议、同一套
断言宏语义）。

## C 用例

1. 新建 `infra/testcases/src/test_<name>.c`：

```c
#include "test_common.h"

static void test_my_feature(void)
{
    printf("\nTest: my feature does the thing\n");

    /* 通过 syscall / ioctl / /proc / /sys / /dev 驱动被测内核 */
    if (/* 期望条件 */)
        PASS("the thing happened");
    else
        FAIL("expected X, got Y");
}

void run_tests(void)
{
    test_my_feature();
}
```

宏：`PASS` / `FAIL` / `SKIP` / `INFO`；不要写自己的 `main()`（共享的会被链入）。

2. 在 `infra/testcases/CMakeLists.txt` 注册（构建为 `-static -O2 -Wall`）：

```cmake
add_executable(test-<name>
    src/main.c
    src/test_common.c
    src/test_<name>.c
)
set_target_properties(test-<name> PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
```

3. 重建并运行：

```bash
virtuoso build && virtuoso test --timeout 30
```

## Rust 用例（no_std）

`infra/testcases/rust/` 是独立 workspace（不进主 workspace 依赖图）：

- `framework/` 是 `testfw` no_std 框架——宏与 C 的 `test_common.h` 一一对齐，
  `run_and_exit` 对齐共享 `main.c` 语义；
- 用例 crate 裸 syscall（write / exit_group）、静态 ELF
  （`-static -nostdlib -nostartfiles -no-pie`），构建产物同样拷入 `/tests/`
  自动发现；
- 骨架参考 `test-rs-example`；
- 宿主缺对应架构 rust-std 时该架构用例显式 WARN 跳过（构建失败仍报错）。

## 标记协议（测试侧）

测试二进制只需输出 `[PASS]` / `[FAIL]` / `[SKIP]` / `[INFO]` 断言行与
`Test Results: N/M passed` 汇总；`--- Running:` / `PASSED:` / `TEST_COMPLETE`
等汇编层标记由 `infra/init` 打印。协议文本冻结，逐条语义见
[冻结契约](../architecture/contracts.md)。

## 需要内核模块的测试

模块清单由组件 `require` 生成，不手写 conf——见
[组件机制](../components/overview.md)。迭代中的模块优先 `=m` 而非 `=y`，
免内核重建。
