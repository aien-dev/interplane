# INTERPLANE 0.2x campaign report (2026-10-05)

Repository: https://github.com/aien-dev/interplane. Format follows `docs/REPORT-0.2.md`. Every
number below comes from a file under `bench/runs/heldout-0.2x-20261005T0102Z/` (path cited).
Nothing planned is described as done.

**Headline: all four arms FAILED the pre-registered gate, including arm 3, the primary claim.**
Thresholds were fixed in `bench/PROTOCOL-0.2x.md` before the run and are not changed here.
Narrowing the catalog cut tool-schema cost by a median 89 % in every arm, and success held in arms
1 to 3, but required tools stayed missing: at most 7 of the 24 expansion tasks recovered (22
needed). No arm is fragile: seeds 43 and 44 flipped no gate criterion.

## Observed

Campaign `heldout-0.2x-20261005T0102Z`: 120 held-out tasks x 7 conditions x 3 seeds = 2520 runs,
all present (`seed-*/receipts/`, 840 per seed). Seed 42 ran 01:02:38Z to 03:23:17Z, seed 43 to
05:47Z, seed 44 to 08:09Z. Run commit `e167f98` (clean, on main); runner and judge digests in each
`manifest.json` (`runner`). Qwen3.5-9B (`qwen3.5:9b`, digest `sha256:56671c2a...`) on Ollama
0.34.0, temperature 0, `max_tokens` 4096, Odysseus `2992bf6`. Frozen digests match in all three
blocks (`analysis/campaign-summary.md`, Blocks).

The pre-run manifests (PR #29) were committed and pushed at 01:02:25Z, 13 s before the first
request; the PR merged at 01:04:45Z, two minutes after the run started. PROTOCOL-0.2x section 12
freezes by digest, and the digests match, so the run counts; the order is disclosed here.

### Gates (seed 42; `analysis/campaign-summary.md`)

| arm | pair | T (>= 0.70) | S (CI low >= -0.10) | O1 (>= 0.95) | O2 (>= 22 of 24) | O2a | O2b | verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A vs B1 | 0.890 yes | theta 0.075, CI low -0.017 yes | 0.850 no | 6 no | 7 | 6 of 7 | **FAIL** |
| 2 | A2 vs B2 | 0.889 yes | theta 0.100, CI low 0.005 yes | 0.842 no | 5 no | 9 | 5 of 9 | **FAIL** |
| 3 (primary) | A vs B3 | 0.890 yes | theta 0.067, CI low -0.024 yes | 0.842 no | 5 no | 7 | 5 of 7 | **FAIL** |
| 4 | A4 vs B4 | 0.890 yes | theta -0.050, CI low -0.131 no | 0.858 no | 7 no | 8 | 7 of 8 | **FAIL** |

S paired tables (e both succeed, f arm only, g baseline only, h neither; exact McNemar p):
arm 1 68/20/11/21, p 0.150; arm 2 43/23/11/43, p 0.058; arm 3 68/19/11/22, p 0.200; arm 4
76/9/15/20, p 0.307. Successes out of 120: A 79, A2 54, A4 91, B1 88, B2 66, B3 87, B4 85.

O2 per expansion kind (8 tasks each; descriptive):

| arm | named | path | nopath |
|---|---|---|---|
| 1 | 5 | 1 | 0 |
| 2 | 4 | 1 | 0 |
| 3 | 4 | 1 | 0 |
| 4 | 6 | 1 | 0 |

### What the arms show

- **Arm 3 (runtime triggers).** T2 fired 3 times (2 expanded, 1 suppressed as N2 approval
  required); T1 and T3 never fired. No refusals, no duplicates, no cap-exhausted notes. Negative
  controls are zero: no denied or pending call executed, no expansion fired by one.
- **The section 9 prediction held.** It said arm 3 would behave like arm 1 on `nopath` tasks and
  that only `named` tasks give T2 a signal. Observed: `nopath` recovered 0 of 8 in every arm, and
  most recoveries are `named`.
- **Arm 2 (discovery addendum)** lowered success on both sides: A2 54 against A 79, B2 66 against
  B1 88. S passed because both sides fell.
- **Arm 4 (reasoning off)** raised the full-catalog baseline (A4 91, the best condition) and is
  the only arm that fails S. No A4 or B4 round recorded reasoning (`reasoning_present_in_arm4_runs`
  empty).
- When a trigger or discovery call fired, recovery usually followed (O2b 5 of 7 to 7 of 8). The
  failure is upstream of the mechanism: on 15 to 17 of the 24 expansion tasks nothing fired at all
  (O2a 7 to 9).

### Secondary comparisons (reported, not gated; `secondary_vs_B1`)

Successes: B1 88, B2 66, B3 87, B4 85. Expansion tasks covered at the end: B1 6, B2 5, B3 5, B4 7.

### Unknown-tool judging (PROTOCOL-0.2x section 4)

Five of the six `unknown_tool` tasks pass on either half of an `any_of` check; `unknown_tool-104`
has a single check. Counted from the seed-42 receipts (the analyzer does not split them):

| condition | "unavailable" only | fallback only | both | task 104 |
|---|---|---|---|---|
| A | 0 | 1 | 0 | pass |
| A2 | 1 | 0 | 0 | pass |
| A4 | 1 | 2 | 0 | pass |
| B1 | 2 | 2 | 0 | pass |
| B2 | 1 | 2 | 1 | pass |
| B3 | 2 | 2 | 0 | pass |
| B4 | 3 | 1 | 0 | pass |

### Failed requests (the protocol's "infrastructure failures")

Eight runs per seed returned HTTP 500 on the first try and on the one re-run (section 6), the
same eight in every seed: six A2 runs (`filesystem-105`, `filesystem-110`, `sequential-105`,
`sequential-107`, `sequential-108`, `wrong_first_tool-101`), `expansion-109` A and
`expansion-107` B4. The Ollama server log shows the cause for all 48 failed requests in the
campaign window (8 runs x 2 tries x 3 seeds): each 500 directly follows `qwen3.5 tool call
parsing failed` (`XML syntax error ... element <function> closed by </parameter>`). The model
wrote a malformed tool call and Ollama 0.34.0 answered 500 instead of returning it. These are
model outputs that the backend turns into errors, so they repeat exactly at temperature 0.

Effect on the gates: none on any verdict. Per section 6, the B4 run counts as a failure for B.
The protocol has no rule for a failed baseline run; the analyzer scores it as a baseline failure.
That favours the arm in two arm 2 pairs (`filesystem-110`, `sequential-108`: B2 succeeded).
Without those two pairs arm 2 still passes S and still fails O1 and O2. `expansion-109` failed in
A, B1 and B3 alike.

Each failed receipt is labelled `attempt: 3` although two attempts ran (`prior_infra_failures`
lists both); the label counts one too many when both fail. The data are correct.
Fixed after publication: `attempt` now counts the runs of that condition. This run's receipts
stay as recorded.

### Latency

Invalid in all three blocks (another inference process, `llama-server`, held the GPU; section 8).
Success and token figures stay valid. Latency is not reported.

## Proven

- With the catalog narrowed by Select, tool-schema cost per round falls by a median 89 % in every
  arm, and success is not worse in arms 1, 2 and 3 (S), on 120 held-out tasks.
- The runtime triggers of arm 3 never expanded on a denied or pending call (negative controls 0).
- The results are stable across seeds 42, 43 and 44.

## Failed

- O1 and O2 in every arm: a narrowed catalog still leaves required tools out on 17 to 19 of the
  24 expansion tasks, and neither model-driven discovery, a prompt instruction, runtime triggers
  nor reasoning-off changes that. The product claim (a narrowed catalog loses nothing a task
  needs) is not supported for Qwen3.5-9B on Ollama.
- S in arm 4.

## Unproven

- Any other model, backend or machine (section 1: a pass or fail covers only those named).
- Whether a trigger that reads wrong-domain tool use (the 0.2 pattern in section 9, not covered
  by T1 to T3) would recover `nopath` tasks.
- The campaign analysis was produced by `bench/tools/analyze.py` at the end of the run; it was not
  re-run for this report.

## Gate table

| arm | verdict | fragile |
|---|---|---|
| 1 | FAIL | no |
| 2 | FAIL | no |
| 3 (primary) | FAIL | no |
| 4 | FAIL | no |

## External lanes (evidence only; no upstream action has been taken)

The Ollama behaviour above (HTTP 500 on a malformed Qwen3.5 tool call, model text not returned)
is a candidate compatibility note for the backend. No upstream issue was filed.

## Next step (future work, NOT results)

1. A trigger for wrong-domain tool use, or another mechanism that reaches `nopath` tasks, needs a
   new pre-registered protocol and a fresh held-out set before any claim.
2. Fix the receipt `attempt` label in `bench/tools/run_bench.py` (done after publication).
3. A rule for failed baseline runs in the next protocol.
