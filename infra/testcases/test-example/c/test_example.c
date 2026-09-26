/*
 * 示例 C 测试体 —— 经 coda-build（build-dep）编入 test-example 二进制。
 * 断言只走 coda.h 的宏（计数/打印与 Rust 侧同源）；协议汇编层前缀
 * （--- Running: / PASSED: / …）由 init 打印，这里不要碰。
 */

#include "coda.h"

static void test_basic(void)
{
    INFO("C section: basic functionality");
    PASS("Basic test passed");
}

void run_c_tests(void)
{
    test_basic();
}
