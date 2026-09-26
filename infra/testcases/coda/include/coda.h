/*
 * coda.h — Virtuoso 测试框架（coda）的 C 侧接口。
 *
 * C 测试体经 FFI 落到 coda（Rust）的统一打印/计数路径：标记协议 v1
 * （冻结文本）单一来源，judge 对 C/Rust 用例一视同仁。宏在预处理层
 * snprintf 预格式化后调 `coda_*`，用法与旧 test_common.h 一致：
 *
 *     #include "coda.h"
 *     PASS("opened %s", path);
 *     FAIL("errno=%d", errno);
 *
 * 注意：断言输出只走本头文件的宏，不要直接 printf 协议前缀
 * （`--- Running:`/`PASSED:`/`FAILED:`/`Test Results:`/`TEST_COMPLETE:`
 * 属于 init 的汇编层）。
 */

#ifndef CODA_H
#define CODA_H

#include <stdio.h>

/* FFI：实现在 coda（Rust staticlib/rlib 链入本二进制）。 */
void coda_pass(const char *msg);
void coda_fail(const char *msg);
void coda_skip(const char *msg);
void coda_info(const char *msg);

/* 计数器（只读快照）：与 coda::counters() 同源。 */
unsigned coda_count_passed(void);
unsigned coda_count_failed(void);
unsigned coda_count_skipped(void);

#define PASS(fmt, ...)                                                       \
    do {                                                                     \
        char _coda_buf[512];                                                 \
        snprintf(_coda_buf, sizeof(_coda_buf), fmt, ##__VA_ARGS__);          \
        coda_pass(_coda_buf);                                                \
    } while (0)

#define FAIL(fmt, ...)                                                       \
    do {                                                                     \
        char _coda_buf[512];                                                 \
        snprintf(_coda_buf, sizeof(_coda_buf), fmt, ##__VA_ARGS__);          \
        coda_fail(_coda_buf);                                                \
    } while (0)

#define SKIP(fmt, ...)                                                       \
    do {                                                                     \
        char _coda_buf[512];                                                 \
        snprintf(_coda_buf, sizeof(_coda_buf), fmt, ##__VA_ARGS__);          \
        coda_skip(_coda_buf);                                                \
    } while (0)

#define INFO(fmt, ...)                                                       \
    do {                                                                     \
        char _coda_buf[512];                                                 \
        snprintf(_coda_buf, sizeof(_coda_buf), fmt, ##__VA_ARGS__);          \
        coda_info(_coda_buf);                                                \
    } while (0)

#endif /* CODA_H */
