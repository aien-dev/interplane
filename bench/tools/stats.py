"""Pre-registered paired statistics for the INTERPLANE 0.2 bench (stdlib only).

Paired success table per qualification task (A = full catalog, B = CrossAxis Select):
  e = both succeed, f = B succeeds and A fails, g = A succeeds and B fails, h = both fail.
Success delta theta = p_B - p_A = (f - g) / n.

* ``newcombe10``: 95% CI for theta, Newcombe RG (1998) "Improved confidence intervals for the
  difference between binomial proportions based on paired data", Statist. Med. 17:2635-2650,
  method 10 (Wilson score limits for each marginal proportion, square-and-add, phi with the
  continuity-corrected numerator max(eh - fg - n/2, 0) when eh > fg; phi = 0 when the
  denominator is 0). ``selftest`` reproduces the method-10 rows of the paper's Table III.
* ``mcnemar_exact``: two-sided exact binomial test on the discordant pairs (f, g), p = 1/2.

Run ``python3 bench/tools/stats.py`` to self-test and print the gate feasibility table.
``python3 bench/tools/stats.py --power`` prints the seeded power simulation behind the sample
size of bench/PROTOCOL-0.2x.md (section 7).
"""

from __future__ import annotations

import math
import random
import sys

Z95 = 1.959963984540054


def wilson(x: int, n: int, z: float = Z95) -> tuple:
    """Wilson score interval (no continuity correction) for x successes out of n."""
    if n == 0:
        return 0.0, 1.0
    p = x / n
    denom = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
    return max(0.0, centre - half), min(1.0, centre + half)


def newcombe10(e: int, f: int, g: int, h: int, z: float = Z95) -> tuple:
    """(theta, lower, upper) for theta = (f - g) / n, Newcombe 1998 method 10."""
    n = e + f + g + h
    if n == 0:
        raise ValueError("empty table")
    p1, p2 = (e + f) / n, (e + g) / n
    l1, u1 = wilson(e + f, n, z)
    l2, u2 = wilson(e + g, n, z)
    num = e * h - f * g
    if num > 0:
        num = max(num - n / 2, 0.0)
    den = math.sqrt((e + f) * (g + h) * (e + g) * (f + h))
    phi = num / den if den > 0 else 0.0
    dl1, du1 = p1 - l1, u1 - p1
    dl2, du2 = p2 - l2, u2 - p2
    d = math.sqrt(max(dl1 ** 2 - 2 * phi * dl1 * du2 + du2 ** 2, 0.0))
    ep = math.sqrt(max(du1 ** 2 - 2 * phi * du1 * dl2 + dl2 ** 2, 0.0))
    theta = p1 - p2
    return theta, max(-1.0, theta - d), min(1.0, theta + ep)


def mcnemar_exact(f: int, g: int) -> float:
    """Two-sided exact McNemar p-value (binomial on discordant pairs, p = 1/2)."""
    m = f + g
    if m == 0:
        return 1.0
    k = min(f, g)
    tail = sum(math.comb(m, i) for i in range(k + 1)) / 2 ** m
    return min(1.0, 2 * tail)


MARGIN = 0.10  # pre-registered success-delta margin (PROTOCOL-0.2.md)


def success_gate(e: int, f: int, g: int, h: int) -> dict:
    """The pre-registered success component of the 0.2 gate (PROTOCOL-0.2.md, section 6)."""
    theta, lo, hi = newcombe10(e, f, g, h)
    p = mcnemar_exact(f, g)
    significantly_worse = p < 0.05 and g > f
    return {
        "theta": theta, "ci95": [lo, hi], "mcnemar_p": p,
        "non_inferior": lo >= -MARGIN and not significantly_worse,
        "equivalent": lo >= -MARGIN and hi <= MARGIN and p >= 0.05,
    }


# Newcombe 1998 Table III, method 10 rows: (e, f, g, h) -> (lower, upper), 4 decimals.
TABLE_III_M10 = [
    ((36, 12, 2, 0), (0.0569, 0.3404)),
    ((20, 12, 2, 16), (0.0562, 0.3292)),
    ((18, 12, 2, 18), (0.0562, 0.3290)),
    ((36, 14, 0, 0), (0.1528, 0.4167)),
    ((35, 14, 0, 1), (0.1461, 0.4175)),
    ((18, 14, 0, 18), (0.1441, 0.3963)),
    ((2, 97, 1, 0), (0.8721, 0.9854)),
    ((1, 97, 1, 1), (0.8736, 0.9850)),
    ((0, 29, 1, 0), (0.6666, 0.9882)),
    ((2, 98, 0, 0), (0.9178, 0.9945)),
    ((1, 98, 0, 1), (0.9171, 0.9916)),
    ((0, 30, 0, 0), (0.8395, 1.0)),
    ((54, 0, 0, 0), (-0.0664, 0.0664)),
]


def selftest() -> list:
    bad = []
    for (e, f, g, h), (lo, hi) in TABLE_III_M10:
        _, l, u = newcombe10(e, f, g, h)
        if abs(l - lo) > 0.00015 or abs(u - hi) > 0.00015:
            bad.append(((e, f, g, h), (round(l, 4), round(u, 4)), (lo, hi)))
    return bad


def feasibility(n: int, max_discordant: int = 4) -> list:
    """Gate outcome for every (f, g) with f + g <= max_discordant at n pairs, worst-case split
    of the concordant pairs (all concordant pairs succeed except ``n_fail_both`` = 2)."""
    rows = []
    for f in range(max_discordant + 1):
        for g in range(max_discordant + 1 - f):
            h = 2
            e = n - f - g - h
            if e < 0:
                continue
            r = success_gate(e, f, g, h)
            rows.append((f, g, r))
    return rows


def power(n: int, discordance: float, theta: float = 0.0, both_succeed: float = 0.75,
          trials: int = 3000, seed: int = 1) -> float:
    """Share of simulated paired runs in which the S criterion (``success_gate`` non_inferior)
    holds. Each of ``n`` pairs is B-only with p = (discordance + theta) / 2, A-only with
    p = (discordance - theta) / 2, both-succeed with ``both_succeed``, else both-fail.
    Seeded, so the table is reproducible byte for byte."""
    if discordance < abs(theta):
        raise ValueError("discordance must be at least |theta|")
    rng = random.Random(seed)
    pf, pg = (discordance + theta) / 2, (discordance - theta) / 2
    hits = 0
    for _ in range(trials):
        e = f = g = h = 0
        for _ in range(n):
            r = rng.random()
            if r < pf:
                f += 1
            elif r < pf + pg:
                g += 1
            elif r < pf + pg + both_succeed:
                e += 1
            else:
                h += 1
        hits += success_gate(e, f, g, h)["non_inferior"]
    return hits / trials


POWER_NS = (60, 80, 100, 120, 150)
POWER_DISCORDANCE = (0.08, 0.12, 0.16, 0.20)


def power_table() -> None:
    print("S criterion pass rate (seed 1, 3000 trials per cell, both-succeed 0.75)")
    print("true delta 0 (power):     n  " + "  ".join(f"pd={d:.2f}" for d in POWER_DISCORDANCE))
    for n in POWER_NS:
        print(f"                       {n:>3}  " + "  ".join(f"{power(n, d):7.3f}" for d in POWER_DISCORDANCE))
    print("true delta -0.10 (false pass rate at the margin):")
    for n in POWER_NS:
        print(f"                       {n:>3}  " + "  ".join(
            f"{power(n, d, theta=-0.10):7.3f}" if d >= 0.10 else "    n/a" for d in POWER_DISCORDANCE))


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "--power":
        power_table()
        return 0
    bad = selftest()
    print(f"newcombe10 vs Newcombe 1998 Table III (method 10): {len(TABLE_III_M10) - len(bad)}/{len(TABLE_III_M10)} rows match")
    for b in bad:
        print("  MISMATCH", b)
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 33
    print(f"gate feasibility at n={n} qualification pairs (h = 2 both-fail, margin {MARGIN:.0%}):")
    print("  f(B only) g(A only)  delta   95% CI            McNemar p  non-inferior  equivalent")
    for f, g, r in feasibility(n):
        lo, hi = r["ci95"]
        print(f"  {f:>8} {g:>9}  {r['theta']:+.3f}  [{lo:+.3f}, {hi:+.3f}]  {r['mcnemar_p']:.3f}      "
              f"{'PASS' if r['non_inferior'] else 'fail':<12}  {'yes' if r['equivalent'] else 'no'}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
