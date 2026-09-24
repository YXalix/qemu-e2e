/*
 * testfw.h — Virtuoso 测试框架的 C 侧接口。
 *
 * C 测试体经 FFI 落到 testfw（Rust）的统一打印/计数路径：标记协议 v1
 * （冻结文本）单一来源，judge 对 C/Rust 用例一视同仁。宏在预处理层
 * snprintf 预格式化后调 `testfw_*`，用法与旧 `test_common.h` 一致：
 *
 *     #include "testfw.h"
 *     PASS("opened %s", path);
 *     FAIL("errno=%d", errno);
 *
 * 注意：断言输出只走本头文件的宏，不要直接 printf 协议前缀
 * （`--- Running:`/`PASSED:`/`FAILED:`/`Test Results:`/`TEST_COMPLETE:`
 * 属于 init 的汇编层）。
 */

#ifndef TESTFW_H
#define TESTFW_H

#include <stdio.h>

/* FFI：实现在 testfw（Rust staticlib/rlib 链入本二进制）。 */
void testfw_pass(const char *msg);
void testfw_fail(const char *msg);
void testfw_skip(const char *msg);
void testfw_info(const char *msg);

/* 计数器（只读快照）：与 testfw::counters() 同源。 */
unsigned testfw_count_passed(void);
unsigned testfw_count_failed(void);
unsigned testfw_count_skipped(void);

#define PASS(fmt, ...)                                                       \
    do {                                                                     \
        char _testfw_buf[512];                                               \
        snprintf(_testfw_buf, sizeof(_testfw_buf), fmt, ##__VA_ARGS__);      \
        testfw_pass(_testfw_buf);                                            \
    } while (0)

#define FAIL(fmt, ...)                                                       \
    do {                                                                     \
        char _testfw_buf[512];                                               \
        snprintf(_testfw_buf, sizeof(_testfw_buf), fmt, ##__VA_ARGS__);      \
        testfw_fail(_testfw_buf);                                            \
    } while (0)

#define SKIP(fmt, ...)                                                       \
    do {                                                                     \
        char _testfw_buf[512];                                               \
        snprintf(_testfw_buf, sizeof(_testfw_buf), fmt, ##__VA_ARGS__);      \
        testfw_skip(_testfw_buf);                                            \
    } while (0)

#define INFO(fmt, ...)                                                       \
    do {                                                                     \
        char _testfw_buf[512];                                               \
        snprintf(_testfw_buf, sizeof(_testfw_buf), fmt, ##__VA_ARGS__);      \
        testfw_info(_testfw_buf);                                            \
    } while (0)

#endif /* TESTFW_H */
