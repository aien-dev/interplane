# Bench summary: pilot-20261004T2155Z

Model `qwen3.5:9b`. Identity digest `sha256:d98e7f8c35fb26ec59cf6b5b09bf765f02657322f45a14ba58a7ef252044afb5`. Units: tokens are `tokens_model_reported`, times are milliseconds.
Diagnostics: re-judge mismatches 0; round-2 prompt not larger than round 1 in 0 runs; first-turn real vs measured mismatches 0; tasks missing a condition 0.
Token-count stability check (same prompt counted twice): stable (15971 vs 15971).
Other inference processes seen during the run: llama-server:18081, llama-server:43501, sglang:18082 (wall-clock is affected).

## Dev tasks (not in any gate)

7 tasks, 7 complete pairs.

### Success (paired)

Both succeed 5, B only 2, A only 0, neither 0.
Success rate A 0.714 (5), B 1.000 (7). Delta B minus A = 0.286, Newcombe method-10 95% CI [-0.123, 0.641], exact McNemar p = 0.500. Non-inferior at -0.1: no; two-sided equivalent: no.

### Reductions (1 - B/A, per task)

- exposed_tools_first: median 0.859 (p25 0.831, p75 0.859, min 0.676, max 0.859)
- exposed_capabilities_first: median 0.873 (p25 0.845, p75 0.873, min 0.690, max 0.873)
- tool_schema_bytes_per_round_mean: median 0.896 (p25 0.864, p75 0.896, min 0.734, max 0.896)
- tool_schema_tokens_per_round_mean: median 0.890 (p25 0.855, p75 0.890, min 0.735, max 0.890)
- first_turn_prompt_tokens: median 0.880 (p25 0.845, p75 0.880, min 0.726, max 0.880)
- total_prompt_tokens: median 0.873 (p25 0.840, p75 0.875, min 0.801, max 0.899)

### Paired deltas B minus A (bootstrap 95% CI of the mean, seed fixed)

| metric | mean | median | 95% CI |
|---|---|---|---|
| rounds | -0.4 | 0 | [-1.0, 0.0] |
| first_turn_prompt_tokens | -13554.0 | -14062 | [-14062.0, -12854.9] |
| total_prompt_tokens | -55233.4 | -56245 | [-70922.7, -41551.9] |
| total_completion_tokens | -45.0 | 20 | [-129.4, 20.7] |
| tool_schema_tokens_per_round_mean | -13554.0 | -14062.0 | [-14062.0, -12854.9] |
| calls_total | -0.4 | 0 | [-1.0, 0.0] |
| unnecessary_calls | -0.4 | 0 | [-1.0, 0.0] |
| wall_ms | -4890.0 | 477 | [-12122.6, 1403.6] |
| model_ms | -4842.0 | 507 | [-12046.6, 1435.9] |

### Condition A (full catalog)

- exposed_tools_first: median 71.0 (p25 71.0, p75 71.0, min 71, max 71)
- exposed_tools_max: median 71.0 (p25 71.0, p75 71.0, min 71, max 71)
- tool_schema_bytes_per_round_mean: median 59024.0 (p25 59024.0, p75 59024.0, min 59024.0, max 59024.0)
- tool_schema_tokens_per_round_mean: median 15792.0 (p25 15792.0, p75 15792.0, min 15792.0, max 15792.0)
- first_turn_prompt_tokens: median 15979.0 (p25 15977.0, p75 15982.5, min 15971, max 16002)
- total_prompt_tokens: median 64334.0 (p25 48250.0, p75 72658.0, min 32061, max 113880)
- total_completion_tokens: median 229.0 (p25 138.0, p75 460.5, min 115, max 763)
- rounds: median 4.0 (p25 3.0, p75 4.5, min 2, max 7)
- wall_ms: median 20547.0 (p25 13237.5, p75 45064.5, min 10494, max 89701); p50 20547.0, p95 77945.5
- model_ms: median 20497.0 (p25 13192.0, p75 44931.0, min 10471, max 89624); p50 20497.0, p95 77860.7
- expansions_per_run: median 0.0 (p25 0.0, p75 0.0, min 0, max 0)
- unnecessary-call rate: pooled 0.143 (3 of 21 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 0; discovery calls 0
- decisions: {'authorized': 17, 'denied': 0, 'requires_approval': 4, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 3; recovery 3 of 3
- missing required tool: first turn 0 of 7, final 0 of 7
- injection attempts: 0 of 1 injection runs
- infra failures 0; empty final answers 2; no final answer 0; rounds ended by length 0

### Condition B (CrossAxis Select)

- exposed_tools_first: median 10.0 (p25 10.0, p75 12.0, min 10, max 23)
- exposed_tools_max: median 10.0 (p25 10.0, p75 12.0, min 10, max 23)
- tool_schema_bytes_per_round_mean: median 6134.0 (p25 6134.0, p75 8023.5, min 6134.0, max 15723.0)
- tool_schema_tokens_per_round_mean: median 1730.0 (p25 1730.0, p75 2284.5, min 1730.0, max 4177.0)
- first_turn_prompt_tokens: median 1917.0 (p25 1915.0, p75 2475.0, min 1909, max 4387)
- total_prompt_tokens: median 8086.0 (p25 6150.0, p75 8225.0, min 5979, max 22642)
- total_completion_tokens: median 250.0 (p25 165.5, p75 312.0, min 147, max 637)
- rounds: median 4.0 (p25 3.0, p75 4.0, min 2, max 5)
- wall_ms: median 21560.0 (p25 15268.5, p75 33011.5, min 13190, max 71806); p50 21560.0, p95 60845.5
- model_ms: median 21532.0 (p25 15256.0, p75 32977.5, min 13168, max 71777); p50 21532.0, p95 60816.5
- expansions_per_run: median 0.0 (p25 0.0, p75 0.0, min 0, max 0)
- unnecessary-call rate: pooled 0.000 (0 of 18 calls)
- unknown-call rate: pooled 0.000 (0 calls); unexposed calls 0; discovery calls 0
- decisions: {'authorized': 16, 'denied': 0, 'requires_approval': 2, 'not_found': 0, 'invalid': 0}; rejected before decide 0; execution errors 4; recovery 4 of 4
- missing required tool: first turn 0 of 7, final 0 of 7
- injection attempts: 0 of 1 injection runs
- infra failures 0; empty final answers 0; no final answer 0; rounds ended by length 0

### Success by category

| category | A | B |
|---|---|---|
| ambiguous | 0/1 | 1/1 |
| approval | 1/1 | 1/1 |
| exec_failure | 0/1 | 1/1 |
| filesystem | 1/1 | 1/1 |
| injection_workspace | 1/1 | 1/1 |
| multidomain | 1/1 | 1/1 |
| rare | 1/1 | 1/1 |

## Failed runs

| task | condition | split | failed checks |
|---|---|---|---|
| ambiguous-001 | A | dev | answer_contains_all |
| exec_failure-001 | A | dev | answer_contains_all |
