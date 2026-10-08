#include <stdio.h>
#include "../src/clamp.h"

static int failures;

static void check(const char *name, int got, int want)
{
    if (got != want) {
        printf("FAIL %s: got %d, want %d\n", name, got, want);
        failures++;
    } else {
        printf("ok   %s\n", name);
    }
}

int main(void)
{
    check("inside", clamp(5, 0, 10), 5);
    check("below", clamp(-3, 0, 10), 0);
    check("above", clamp(42, 0, 10), 10);
    check("at_low", clamp(0, 0, 10), 0);
    check("at_high", clamp(10, 0, 10), 10);
    return failures ? 1 : 0;
}
