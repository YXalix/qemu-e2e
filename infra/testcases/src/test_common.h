#ifndef TEST_COMMON_H
#define TEST_COMMON_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>

/* Test result counters */
extern int passed;
extern int failed;
extern int skipped;

#define PASS(fmt, ...) do { passed++; printf("  [PASS] " fmt "\n", ##__VA_ARGS__); } while(0)
#define FAIL(fmt, ...) do { failed++; printf("  [FAIL] " fmt "\n", ##__VA_ARGS__); } while(0)
#define SKIP(fmt, ...) do { skipped++; printf("  [SKIP] " fmt "\n", ##__VA_ARGS__); } while(0)
#define INFO(fmt, ...) do { printf("  [INFO] " fmt "\n", ##__VA_ARGS__); } while(0)

/* Test entry point - to be implemented by specific test files */
void run_tests(void);

#endif
