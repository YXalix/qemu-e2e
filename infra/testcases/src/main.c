#include <stdio.h>
#include <stdlib.h>
#include "test_common.h"

int main(int argc, char *argv[])
{
    (void)argc;
    (void)argv;

    printf("=== QEMU E2E Tests ===\n");
    printf("Kernel: ");
    fflush(stdout);
    system("uname -r");

    if (getuid() != 0)
        printf("WARNING: Not running as root\n");

    /* Run tests defined by the specific test program */
    run_tests();

    printf("\n=== Summary ===\n");
    printf("  Passed:  %d\n", passed);
    printf("  Failed:  %d\n", failed);
    printf("  Skipped: %d\n", skipped);

    return failed > 0 ? 1 : 0;
}
