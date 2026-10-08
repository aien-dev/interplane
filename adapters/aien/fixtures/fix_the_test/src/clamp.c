#include "clamp.h"

/* Limit v to the range lo..hi. */
int clamp(int v, int lo, int hi)
{
    if (v < lo)
        return lo;
    if (v > hi)
        return lo;
    return v;
}
