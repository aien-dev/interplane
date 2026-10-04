# Bench summary: synthetic-run

Model `synthetic`. Identity digest `sha256:synthetic`. Units: tokens are `tokens_model_reported`, times are milliseconds.
Diagnostics: re-judge mismatches 0; round-2 prompt not larger than round 1 in 0 runs; first-turn real vs measured mismatches 0; tasks missing a condition 0.
Token-count stability check (same prompt counted twice): stable (1 vs 1).

## Qualification tasks (gate set)

8 tasks, 8 complete pairs.

### Success (paired)

Both succeed 5, B only 1, A only 1, neither 1.
Success rate A 0.750 (6), B 0.750 (6). Delta B minus A = 0.000, Newcombe method-10 95% CI [-0.385, 0.385], exact McNemar p = 1.000. Non-inferior at -0.1: no; two-sided equivalent: no.

### Gate (pre-registered, PROTOCOL-0.2.md section 6)

Overall: **INCOMPLETE** (8 of 37 qualification pairs).

| criterion | value | threshold | result |
|---|---|---|---|
| T median tool-schema token reduction | 0.915 (n=8) | >= 0.7 | yes |
| S success not worse | theta 0.000, CI lower -0.385, McNemar p 1.000 | CI lower >= -0.10 and not (p<0.05 with g>f) | no |
| O1 required tools exposed (final) | 1.000 covered; uncovered none | >= 0.95 | yes |
| O2 recovered within 1 expansion | 2 of 2 (1.000); not recovered none | >= 0.9 | yes |

### Reductions (1 - B/A, per task)

- exposed_tools_first: median 0.859 (p25 0.859, p75 0.859, min 0.859, max 0.859)
- exposed_capabilities_first: median 0.873 (p25 0.873, p75 0.873, min 0.873, max 0.873)
- tool_schema_bytes_per_round_mean: median 0.898 (p25 0.898, p75 0.898, min 0.898, max 0.898)
- tool_schema_tokens_per_round_mean: median 0.915 (p25 0.913, p75 0.915, min 0.913, max 0.917)
- first_turn_prompt_tokens: median 0.892 (p25 0.892, p75 0.892, min 0.892, max 0.892)
- total_prompt_tokens: median 0.890 (p25 0.884, p75 0.901, min 0.825, max 0.926)

### Paired deltas B minus A (bootstrap 95% CI of the mean, seed fixed)

| metric | mean | median | 95% CI |
|---|---|---|---|
| rounds | -0.1 | 0.0 | [-0.5, 0.2] |
| first_turn_prompt_tokens | -14100.0 | -14100.0 | [-14100.0, -14100.0] |
| total_prompt_tokens | -31975.0 | -35250.0 | [-39775.0, -23162.5] |
| total_completion_tokens | -25.9 | -28.0 | [-62.8, 8.5] |
| tool_schema_tokens_per_round_mean | -8597.5 | -8600.0 | [-8607.5, -8587.5] |
| calls_total | 0.0 | 0.0 | [0.0, 0.0] |
| unnecessary_calls | 0.0 | 0.0 | [0.0, 0.0] |
| wall_ms | -1295.4 | -1588.5 | [-2698.8, 198.0] |
| model_ms | -1295.4 | -1588.5 | [-2698.8, 198.0] |

### Condition A (full catalog)

- exposed_tools_first: median 71.0 (p25 71.0, p75 71.0, min 71, max 71)
- exposed_tools_max: median 71.0 (p25 71.0, p75 71.0, min 71, max 71)
- tool_schema_bytes_per_round_mean: median 59000.0 (p25 59000.0, p75 59000.0, min 59000.0, max 59000.0)
- tool_schema_tokens_per_round_mean: median 9400.0 (p25 9400.0, p75 9400.0, min 9400.0, max 9400.0)
- first_turn_prompt_tokens: median 15800.0 (p25 15800.0, p75 15800.0, min 15800, max 15800)
- total_prompt_tokens: median 39800.0 (p25 27762.5, p75 47850.0, min 15800, max 47850)
- total_completion_tokens: median 93.0 (p25 74.5, p75 156.5, min 65, max 226)
- rounds: median 2.5 (p25 1.8, p75 3.0, min 1, max 3)
- wall_ms: median 6335.5 (p25 3491.5, p75 7586.8, min 2662, max 8829); p50 6335.5, p95 8500.0
- model_ms: median 6285.5 (p25 3441.5, p75 7536.8, min 2612, max 8779); p50 6285.5, p95 8450.0
- expansions_per_run: median 0.0 (p25 0.0, p75 0.0, min 0, max 0)
- unnecessary-call rate: pooled 0.000 (0 of 8 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 0; discovery calls 0
- decisions: {'authorized': 8, 'denied': 0, 'requires_approval': 0, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 0; recovery 0 of 0
- missing required tool: first turn 0 of 8, final 8 of 8
- injection attempts: 0 of 0 injection runs
- infra failures 0; empty final answers 0; no final answer 0; rounds ended by length 0

### Condition B (CrossAxis Select)

- exposed_tools_first: median 10.0 (p25 10.0, p75 10.0, min 10, max 10)
- exposed_tools_max: median 10.0 (p25 10.0, p75 10.2, min 10, max 11)
- tool_schema_bytes_per_round_mean: median 6000.0 (p25 6000.0, p75 6000.0, min 6000.0, max 6000.0)
- tool_schema_tokens_per_round_mean: median 800.0 (p25 795.0, p75 820.0, min 780.0, max 820.0)
- first_turn_prompt_tokens: median 1700.0 (p25 1700.0, p75 1700.0, min 1700, max 1700)
- total_prompt_tokens: median 3550.0 (p25 3087.5, p75 5550.0, min 1700, max 5550)
- total_completion_tokens: median 102.0 (p25 67.2, p75 118.2, min 29, max 155)
- rounds: median 2.0 (p25 1.8, p75 3.0, min 1, max 3)
- wall_ms: median 4447.5 (p25 3156.5, p75 6364.0, min 1321, max 6478); p50 4447.5, p95 6457.0
- model_ms: median 4397.5 (p25 3106.5, p75 6314.0, min 1271, max 6428); p50 4397.5, p95 6407.0
- expansions_per_run: median 0.0 (p25 0.0, p75 0.2, min 0, max 1)
- unnecessary-call rate: pooled 0.000 (0 of 8 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 2; discovery calls 0
- decisions: {'authorized': 8, 'denied': 0, 'requires_approval': 0, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 0; recovery 0 of 0
- missing required tool: first turn 2 of 8, final 0 of 8
- injection attempts: 0 of 0 injection runs
- infra failures 0; empty final answers 0; no final answer 0; rounds ended by length 0

### Success by category

| category | A | B |
|---|---|---|
| ambiguous | 1/1 | 1/1 |
| expansion | 1/2 | 2/2 |
| filesystem | 2/2 | 2/2 |
| rare | 1/1 | 0/1 |
| unknown_tool | 1/1 | 1/1 |
| wrong_first_tool | 0/1 | 0/1 |

## Dev tasks (not in any gate)

1 tasks, 1 complete pairs.

### Success (paired)

Both succeed 1, B only 0, A only 0, neither 0.
Success rate A 1.000 (1), B 1.000 (1). Delta B minus A = 0.000, Newcombe method-10 95% CI [-0.793, 0.793], exact McNemar p = 1.000. Non-inferior at -0.1: no; two-sided equivalent: no.

### Reductions (1 - B/A, per task)

- exposed_tools_first: median 0.859 (p25 0.859, p75 0.859, min 0.859, max 0.859)
- exposed_capabilities_first: median 0.873 (p25 0.873, p75 0.873, min 0.873, max 0.873)
- tool_schema_bytes_per_round_mean: median 0.898 (p25 0.898, p75 0.898, min 0.898, max 0.898)
- tool_schema_tokens_per_round_mean: median 0.915 (p25 0.915, p75 0.915, min 0.915, max 0.915)
- first_turn_prompt_tokens: median 0.892 (p25 0.892, p75 0.892, min 0.892, max 0.892)
- total_prompt_tokens: median 0.926 (p25 0.926, p75 0.926, min 0.926, max 0.926)

### Paired deltas B minus A (bootstrap 95% CI of the mean, seed fixed)

| metric | mean | median | 95% CI |
|---|---|---|---|
| rounds | -1.0 | -1 | [-1.0, -1.0] |
| first_turn_prompt_tokens | -14100.0 | -14100 | [-14100.0, -14100.0] |
| total_prompt_tokens | -44300.0 | -44300 | [-44300.0, -44300.0] |
| total_completion_tokens | -125.0 | -125 | [-125.0, -125.0] |
| tool_schema_tokens_per_round_mean | -8600.0 | -8600.0 | [-8600.0, -8600.0] |
| calls_total | 0.0 | 0 | [0.0, 0.0] |
| unnecessary_calls | 0.0 | 0 | [0.0, 0.0] |
| wall_ms | -4778.0 | -4778 | [-4778.0, -4778.0] |
| model_ms | -4778.0 | -4778 | [-4778.0, -4778.0] |

### Condition A (full catalog)

- exposed_tools_first: median 71 (p25 71, p75 71, min 71, max 71)
- exposed_tools_max: median 71 (p25 71, p75 71, min 71, max 71)
- tool_schema_bytes_per_round_mean: median 59000.0 (p25 59000.0, p75 59000.0, min 59000.0, max 59000.0)
- tool_schema_tokens_per_round_mean: median 9400.0 (p25 9400.0, p75 9400.0, min 9400.0, max 9400.0)
- first_turn_prompt_tokens: median 15800 (p25 15800, p75 15800, min 15800, max 15800)
- total_prompt_tokens: median 47850 (p25 47850, p75 47850, min 47850, max 47850)
- total_completion_tokens: median 207 (p25 207, p75 207, min 207, max 207)
- rounds: median 3 (p25 3, p75 3, min 3, max 3)
- wall_ms: median 8952 (p25 8952, p75 8952, min 8952, max 8952); p50 8952, p95 8952
- model_ms: median 8902 (p25 8902, p75 8902, min 8902, max 8902); p50 8902, p95 8902
- expansions_per_run: median 0 (p25 0, p75 0, min 0, max 0)
- unnecessary-call rate: pooled 0.000 (0 of 1 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 0; discovery calls 0
- decisions: {'authorized': 1, 'denied': 0, 'requires_approval': 0, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 0; recovery 0 of 0
- missing required tool: first turn 0 of 1, final 1 of 1
- injection attempts: 0 of 0 injection runs
- infra failures 0; empty final answers 0; no final answer 0; rounds ended by length 0

### Condition B (CrossAxis Select)

- exposed_tools_first: median 10 (p25 10, p75 10, min 10, max 10)
- exposed_tools_max: median 10 (p25 10, p75 10, min 10, max 10)
- tool_schema_bytes_per_round_mean: median 6000.0 (p25 6000.0, p75 6000.0, min 6000.0, max 6000.0)
- tool_schema_tokens_per_round_mean: median 800.0 (p25 800.0, p75 800.0, min 800.0, max 800.0)
- first_turn_prompt_tokens: median 1700 (p25 1700, p75 1700, min 1700, max 1700)
- total_prompt_tokens: median 3550 (p25 3550, p75 3550, min 3550, max 3550)
- total_completion_tokens: median 82 (p25 82, p75 82, min 82, max 82)
- rounds: median 2 (p25 2, p75 2, min 2, max 2)
- wall_ms: median 4174 (p25 4174, p75 4174, min 4174, max 4174); p50 4174, p95 4174
- model_ms: median 4124 (p25 4124, p75 4124, min 4124, max 4124); p50 4124, p95 4124
- expansions_per_run: median 0 (p25 0, p75 0, min 0, max 0)
- unnecessary-call rate: pooled 0.000 (0 of 1 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 0; discovery calls 0
- decisions: {'authorized': 1, 'denied': 0, 'requires_approval': 0, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 0; recovery 0 of 0
- missing required tool: first turn 0 of 1, final 0 of 1
- injection attempts: 0 of 0 injection runs
- infra failures 0; empty final answers 0; no final answer 0; rounds ended by length 0

### Success by category

| category | A | B |
|---|---|---|
| filesystem | 1/1 | 1/1 |

## Failed runs

| task | condition | split | failed checks |
|---|---|---|---|
| expansion-001 | A | qual | answer_contains_all |
| rare-002 | B | qual | answer_contains_all, answer_regex |
| wrong_first_tool-001 | A | qual | answer_regex |
| wrong_first_tool-001 | B | qual | answer_regex |
