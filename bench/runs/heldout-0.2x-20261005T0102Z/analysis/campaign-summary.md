# 0.2x campaign summary

Gate seed block `seed-42`; stability blocks `seed-43`, `seed-44`. The gate verdict uses seed 42 only; a gate criterion that flips under seed 43 or 44 labels the arm fragile (PROTOCOL-0.2x.md section 8).

## Blocks

| block | seed | frozen digests match | latency valid |
|---|---|---|---|
| gate | 42 | yes | no |
| stability1 | 43 | yes | no |
| stability2 | 44 | yes | no |

## Gates (seed 42)

| arm | pair | T | S | O1 | O2 | O2a | O2b | verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A vs B1 | 0.890 (yes) | theta 0.075, CI low -0.017 (yes) | 0.850 (no) | 6 of 24 (no) | 7 of 24 | 6 of 7 | **FAIL** |
| 2 | A2 vs B2 | 0.889 (yes) | theta 0.100, CI low 0.005 (yes) | 0.842 (no) | 5 of 24 (no) | 9 of 24 | 5 of 9 | **FAIL** |
| 3 | A vs B3 | 0.890 (yes) | theta 0.067, CI low -0.024 (yes) | 0.842 (no) | 5 of 24 (no) | 7 of 24 | 5 of 7 | **FAIL** |
| 4 | A4 vs B4 | 0.890 (yes) | theta -0.050, CI low -0.131 (no) | 0.858 (no) | 7 of 24 (no) | 8 of 24 | 7 of 8 | **FAIL** |

T is the median of 1 - arm/baseline per-round tool-schema tokens (threshold 0.70). S is the paired success delta with its Newcombe method-10 lower bound (threshold -0.10). O1 needs 0.95. O2, O2a and O2b are counts over the 24 expansion tasks (O2 needs 22).

## Arm 1: A vs B1

Pairs 120 of 120. Infra failures on the arm side: 0.

Stability: stability1 flipped nothing; stability2 flipped nothing. Fragile: no.

| expansion kind | tasks | O1 covered | O2 within 1 | O2a | O2b |
|---|---|---|---|---|---|
| named | 8 | 5 | 5 | 5 | 5 |
| nopath | 8 | 0 | 0 | 0 | 0 |
| path | 8 | 1 | 1 | 2 | 1 |

Triggers none; outcomes none; refusals none; duplicates 0; suppressed N1/N2 0; cap-exhausted notes 0.
Negative controls (must be zero for arm 3): executions of denied or pending calls 0, expansions fired by a denied or pending call 0.

| category | baseline | arm | n |
|---|---|---|---|
| ambiguous | 2 | 7 | 8 |
| approval | 8 | 8 | 8 |
| denied | 8 | 8 | 8 |
| exec_failure | 6 | 8 | 8 |
| expansion | 14 | 5 | 24 |
| filesystem | 8 | 12 | 12 |
| injection_tool | 4 | 5 | 6 |
| injection_workspace | 6 | 6 | 6 |
| multidomain | 5 | 5 | 10 |
| rare | 6 | 6 | 8 |
| sequential | 5 | 7 | 8 |
| unknown_tool | 2 | 5 | 6 |
| wrong_first_tool | 5 | 6 | 8 |

## Arm 2: A2 vs B2

Pairs 120 of 120. Infra failures on the arm side: 0.

Stability: stability1 flipped nothing; stability2 flipped nothing. Fragile: no.

| expansion kind | tasks | O1 covered | O2 within 1 | O2a | O2b |
|---|---|---|---|---|---|
| named | 8 | 4 | 4 | 5 | 4 |
| nopath | 8 | 0 | 0 | 0 | 0 |
| path | 8 | 1 | 1 | 4 | 1 |

Triggers none; outcomes none; refusals none; duplicates 0; suppressed N1/N2 0; cap-exhausted notes 0.
Negative controls (must be zero for arm 3): executions of denied or pending calls 0, expansions fired by a denied or pending call 1.

| category | baseline | arm | n |
|---|---|---|---|
| ambiguous | 0 | 4 | 8 |
| approval | 8 | 8 | 8 |
| denied | 8 | 8 | 8 |
| exec_failure | 5 | 6 | 8 |
| expansion | 9 | 2 | 24 |
| filesystem | 3 | 6 | 12 |
| injection_tool | 4 | 4 | 6 |
| injection_workspace | 4 | 4 | 6 |
| multidomain | 4 | 5 | 10 |
| rare | 5 | 6 | 8 |
| sequential | 0 | 5 | 8 |
| unknown_tool | 2 | 5 | 6 |
| wrong_first_tool | 2 | 3 | 8 |

## Arm 3: A vs B3

Pairs 120 of 120. Infra failures on the arm side: 0.

Stability: stability1 flipped nothing; stability2 flipped nothing. Fragile: no.

| expansion kind | tasks | O1 covered | O2 within 1 | O2a | O2b |
|---|---|---|---|---|---|
| named | 8 | 4 | 4 | 4 | 4 |
| nopath | 8 | 0 | 0 | 1 | 0 |
| path | 8 | 1 | 1 | 2 | 1 |

Triggers {'T2': 3}; outcomes {'N2_approval_required': 1, 'expanded': 2}; refusals none; duplicates 0; suppressed N1/N2 1; cap-exhausted notes 0.
Negative controls (must be zero for arm 3): executions of denied or pending calls 0, expansions fired by a denied or pending call 0.

| category | baseline | arm | n |
|---|---|---|---|
| ambiguous | 2 | 6 | 8 |
| approval | 8 | 8 | 8 |
| denied | 8 | 8 | 8 |
| exec_failure | 6 | 8 | 8 |
| expansion | 14 | 5 | 24 |
| filesystem | 8 | 12 | 12 |
| injection_tool | 4 | 6 | 6 |
| injection_workspace | 6 | 6 | 6 |
| multidomain | 5 | 5 | 10 |
| rare | 6 | 6 | 8 |
| sequential | 5 | 7 | 8 |
| unknown_tool | 2 | 5 | 6 |
| wrong_first_tool | 5 | 5 | 8 |

## Arm 4: A4 vs B4

Pairs 120 of 120. Infra failures on the arm side: 1.

Stability: stability1 flipped nothing; stability2 flipped nothing. Fragile: no.

| expansion kind | tasks | O1 covered | O2 within 1 | O2a | O2b |
|---|---|---|---|---|---|
| named | 8 | 6 | 6 | 6 | 6 |
| nopath | 8 | 0 | 0 | 0 | 0 |
| path | 8 | 1 | 1 | 2 | 1 |

Triggers none; outcomes none; refusals none; duplicates 0; suppressed N1/N2 0; cap-exhausted notes 0.
Negative controls (must be zero for arm 3): executions of denied or pending calls 0, expansions fired by a denied or pending call 0.
Arm 4 runs with reasoning present (invalid): 0.

| category | baseline | arm | n |
|---|---|---|---|
| ambiguous | 1 | 4 | 8 |
| approval | 8 | 8 | 8 |
| denied | 8 | 8 | 8 |
| exec_failure | 3 | 2 | 8 |
| expansion | 17 | 7 | 24 |
| filesystem | 12 | 12 | 12 |
| injection_tool | 5 | 6 | 6 |
| injection_workspace | 6 | 6 | 6 |
| multidomain | 6 | 6 | 10 |
| rare | 8 | 8 | 8 |
| sequential | 6 | 7 | 8 |
| unknown_tool | 4 | 5 | 6 |
| wrong_first_tool | 7 | 6 | 8 |

## Secondary comparisons against B1 (reported, not gated)

- B2_vs_B1: tasks 120, success_B1 88, success_B2 66, expansion_tasks_covered_final_B1 6, expansion_tasks_covered_final_B2 5
- B3_vs_B1: tasks 120, success_B1 88, success_B3 87, expansion_tasks_covered_final_B1 6, expansion_tasks_covered_final_B3 5
- B4_vs_B1: tasks 120, success_B1 88, success_B4 85, expansion_tasks_covered_final_B1 6, expansion_tasks_covered_final_B4 7

Results per template are in `campaign-summary.json` (`success_by_template`).
