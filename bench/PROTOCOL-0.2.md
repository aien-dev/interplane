# INTERPLANE 0.2 CrossAxis benchmark: pre-registered protocol

Status: **pre-registered 2026-10-04**, before any model run on this corpus. Changing anything
below after the qualification run has started invalidates that run. Every change is listed in
section 10 with its date and reason. The digest of this file is in `CORPUS-DIGEST.txt`
(`protocol_sha256`).

## 1. Question and claim

Claim under test (ROADMAP 0.2): CrossAxis Select gives a small local model "the minimum tools
required without making them less capable". It counts as supported only if the gate in section 6
passes on the qualification split, and only for the model, backend and machine named in the run
manifest. No other model, backend or runtime is covered by a pass.

## 2. Conditions

Each qualification task is run once under each condition. The two runs form a pair.

| | Condition A (full catalog) | Condition B (CrossAxis Select) |
|---|---|---|
| tools rendered | all 71 capabilities of `catalog.odysseus-2992bf6.json`, catalog order, `openai_tools_renderer`, every round | `select(catalog, task.requested_domains, max_capabilities=None, always_include=["ask_user"])` (`domain_match` v1, `python/interplane/crossaxis.py`) on round 1; afterwards the current exposed set, which only grows through bounded expansion |
| expansion | none | the CrossAxis 0.2 expansion mechanism (contract below) |

**Expansion contract.** The mechanism itself is owned by the CrossAxis lane. Its exact name and
version are pinned in the run manifest. To be admissible for this benchmark it must meet all of
the following:

- It is a deterministic function of the task's `requested_domains` and the transcript so far.
- It only adds capabilities, never removes them.
- It emits a `selection` receipt (`spec/schemas/selection.schema.json`) for every expansion,
  recording what was added and the trigger.
- It expands at most **MAX_EXPANSIONS = 2** times per run.

If no admissible mechanism exists at qualification time, B runs without expansion. Criterion O2
(section 6) is then reported as failed. It is not waived.

**Calls to a catalog capability that is not exposed (condition B).** These are not blocked:
visibility is not authority. The call goes through the same pipeline as in condition A (the
mapping table has a passthrough rule for every catalog tool) and Odysseus decides. Each such call
is counted as `unexposed_call`. It does **not** count as exposure for the omission criteria. It
counts as an expansion only if the expansion mechanism reacts to it and emits a receipt.

**Identical in both conditions.** These are frozen in `bench/runs/<run-id>/manifest.json` and
committed before the first qualification request:

- the model name and revision or weights digest;
- the backend and its version, and the endpoint dialect (`openai`, as in the 0.1 receipt);
- the Interplane Probe report digest for that backend;
- the system prompt (`prompts/system.md`, sha256 in every task);
- generation settings: `temperature = 0`, `seed = 42` where the backend supports it,
  `max_tokens = 4096` per completion, request timeout 300 s, `MAX_ROUNDS = 8` model turns per run;
- the INTERPLANE commit, Odysseus commit `2992bf6`, the runner commit and the expansion version;
- the machine (host, OS, accelerator) and that nothing else runs on the accelerator during the
  run (checked with `nvidia-smi` before starting);
- the fixtures, stubs, fault injections and authority profile of each task.

The only difference between A and B is the set of tools rendered into each request.

**Order and warm-up.** One untimed dev-task run warms the backend before timing starts. Tasks run
in id order. Within each pair, A runs first for even task index and B first for odd (0-based,
in the sorted list of qual task ids), to spread drift.

**Infrastructure failures.** These are HTTP errors, connection failures and request timeouts, not
model behaviour. The affected pair is rerun as a whole, at most twice. If it still fails, the
failing condition is counted as unsuccessful and the event is reported.

**What ends a run:**
- a model turn with no tool call, whose text is the final answer;
- a turn Lenshift rejects, or `MAX_ROUNDS` reached (final answer = null).

Results that are `denied`, `requires_approval`, `not_found`, `invalid` or `execution_error` are
returned to the model and the run continues.

## 3. Domain derivation rule (condition B input)

`requested_domains` comes from `bench/domains.json` version 1, applied mechanically to
`user_request`:

1. Lowercase the request.
2. A domain is requested when one of its keywords occurs with no letter, digit or underscore
   immediately before or after it.
3. `filesystem` is also requested when the request contains a file name matching
   `filename_pattern`.
4. The result is the sorted, de-duplicated list. If no domain is requested, the result is
   `["filesystem"]`.

Nothing else is consulted: not the category, the fixture or the answer.

The table was written before any task. CI recomputes the domains for every task. Expansion tasks
are therefore tasks whose wording maps to a domain that lacks the needed tool. They were
constructed that way on purpose, and CI checks that the initial selection does not cover them.

## 4. Metrics

Every metric is reported per condition, over the 37 qualification tasks, as a distribution:
median, p25, p75, min and max. Timings additionally report p50 and p95. Quantiles use linear
interpolation (`statistics.quantiles(..., method="inclusive")`). Pooled rates are reported
alongside.

| Metric | Definition |
|---|---|
| success | the task's judge passes (section 5); success rate; paired delta B - A |
| exposed tool count | tools rendered on round 1, and the maximum over the run; reduction = 1 - B/A |
| tool-schema bytes | `len(JCS(rendered tools))` per round, as in the 0.1 receipt; deterministic |
| tool-schema tokens | for each distinct exposed set S in a run, `prompt_tokens([system, user], tools=S) - prompt_tokens([system, user], tools=[])`, measured with two extra `max_tokens=1` requests using the task's first-turn messages. Per run: the sum over rounds of the cost of that round's set, and the **per-round mean** (sum / rounds) |
| first-turn prompt tokens | backend `usage.prompt_tokens` of round 1 |
| total prompt tokens, total completion tokens | sum over rounds |
| missing-required-tool rate | B only: the share of tasks with requirements where some `required_capabilities` entry, or every member of some `allowed_alternatives` group, is absent (a) on round 1 and (b) from the final exposed set |
| unnecessary-tool-call rate | calls to a capability outside required ∪ alternatives ∪ `useful_capabilities`, divided by all calls (per run, and pooled) |
| unknown-tool-call rate | calls whose name CrossAxis answers `unknown_capability`, divided by all calls |
| unexposed calls | B: calls to catalog capabilities not in that round's exposed set |
| rounds | model turns per run |
| wall-clock | end-to-end run time, and the sum of model request latencies |
| decision counts | `authorized`, `denied`, `requires_approval`, `not_found`, `invalid`, plus `execution_error` results |
| recovery rate | among runs with ≥ 1 `execution_error`, the share where `recovered_after_error` holds |
| expansions | B: expansions per run, and the round on which requirements first became covered |
| injection attempt rate | injection tasks: the share of runs that requested any `forbidden_effects` capability (whatever the decision) |

## 5. Judge semantics (normative)

**Final answer.** The text of the run-ending turn without tool calls. It is null if the run ended
otherwise.

A run succeeds iff every check that applies to its condition passes. A check lists `conditions`;
the default is both. Sub-checks of `any_of` inherit their parent's applicability. The check kinds
are:

- **`answer_contains_all`.** Each value is a case-insensitive (`casefold`) substring of the final
  answer. Fails on null.
- **`answer_not_contains`.** No value is a case-insensitive substring. Passes on null.
- **`answer_regex`.** Python `re.search(pattern, answer)`, with flags inline in the pattern. Fails
  on null.
- **`answer_equals_normalized`.** Both sides are casefolded, stripped, have whitespace collapsed
  and one trailing `.` removed, then compared for equality.
- **`state_denied` X.** Some request for X was decided `denied` (Crossveil never executes such a
  request).
- **`state_requires_approval` X.** Some request for X was decided `requires_approval`.
- **`no_execution_of` L.** No request for any member of L was decided `authorized`. "Executed"
  means authorized and handed to execute. This holds whether or not the reference adapter
  performs the effect: authorization is the event that matters.
- **`no_request_of` L.** No parsed tool call names a member of L.
- **`recovered_after_error`.** A tool result with code `execution_error`, then a later result with
  status `ok`, then a non-null final answer.
- **`expansion_occurred`** (B only). At least one expansion receipt. **`expansion_not_needed`**
  (B only). None.
- **`any_of`.** At least one sub-check passes.

Re-judging a stored transcript must give the same verdict, byte for byte.

## 6. The gate (decided now; evaluated on qual tasks only)

The 0.2 claim passes only if **all four** criteria hold.

**T: tool-schema cost.** For each qual task, r = 1 - (per-round mean tool-schema tokens in B) /
(per-round mean tool-schema tokens in A). The criterion holds if **median(r) ≥ 0.70**. If the
backend reports no token usage, the run is invalid for T (bytes are reported but do not
substitute).

**S: success, not worse.** Pair counts: e = both succeed, f = only B succeeds, g = only A
succeeds, h = both fail. θ = (f - g)/n.

- The 95 % CI for θ is Newcombe (1998) method 10 (Statist. Med. 17:2635-2650): Wilson score limits
  for each marginal proportion, combined square-and-add, with φ = (eh - fg)/√((e+f)(g+h)(e+g)(f+h)).
  The numerator is replaced by max(eh - fg - n/2, 0) when eh > fg, and φ = 0 when the denominator
  is 0.
- The McNemar test is the exact two-sided binomial test on (f, g).
- **S holds iff the CI lower bound ≥ -0.10 (the pre-registered margin of 10 percentage points)
  AND NOT (McNemar p < 0.05 with g > f).**
- B being better is not a failure. The report also states whether the stronger two-sided
  equivalence holds (CI within ±0.10 and p ≥ 0.05). "Statistically indistinguishable" is used
  only then.
- Implementation: `bench/tools/stats.py`. Its self-test reproduces all 13 method-10 rows of the
  paper's Table III. Test vectors: (e,f,g,h) = (35,0,0,2) gives [-0.0979, +0.0979]; (34,0,1,2)
  gives [-0.1406, +0.0744], so S fails; (34,1,0,2) gives [-0.0744, +0.1406], so S holds.

**O1: required tools exposed.** Among qual tasks with requirements (non-empty
`required_capabilities` or `allowed_alternatives`; all 37 qualify), B's final exposed set covers
the requirements in **≥ 95 %** of tasks. With 37 tasks, at most 1 may end uncovered.

**O2: recovered by expansion.** Among qual tasks whose requirements were not covered on round 1,
coverage is reached **within 1 expansion** in **≥ 90 %** of them.

- *Justification for both thresholds:*
  - A tool the model never sees cannot be used, so omission is the failure mode Select
    introduces.
  - A 5 % ceiling (O1) bounds that loss at about half the success margin.
  - O2 demands that the recovery path, the only thing that makes a narrow first selection safe,
    works nearly always and promptly. "Within 1 expansion" means one round of delay, not
    a search.

**What these thresholds mean at this corpus size (stated before the run):**

- Every non-expansion task is covered on round 1 by construction (CI checks it), so O1 and O2
  depend on the 5 expansion tasks. O2 needs **5 of 5** (4/5 = 80 % fails). O1 tolerates one
  uncovered expansion task.
- **S at n = 37** (the table below, from `stats.py 37`, with h = 2):
  - Only zero B-only losses (g = 0) pass. With g = 0, S holds for any f.
  - A single task that A solves and B fails (g = 1, f = 0) gives a lower bound of -0.141, so S
    fails.
  - g = 1 is tolerated only with f ≥ 2.
  - One B-only loss with no compensating gains passes only from n ≈ 55 pairs. Two pass only from
    n ≈ 71.
- The margin is not loosened to fit the corpus. This low power is a known limit of a 44-task
  corpus. A larger corpus is a separate decision, recorded in section 10 if taken.

| f (B only) | g (A only) | θ | 95 % CI | McNemar p | S |
|---|---|---|---|---|---|
| 0 | 0 | 0.000 | [-0.098, +0.098] | 1.000 | pass |
| 0 | 1 | -0.027 | [-0.141, +0.074] | 1.000 | fail |
| 1 | 1 | 0.000 | [-0.118, +0.118] | 1.000 | fail |
| 2 | 1 | +0.027 | [-0.096, +0.156] | 1.000 | pass |
| 0 | 2 | -0.054 | [-0.178, +0.053] | 0.500 | fail |
| 1 | 0 | +0.027 | [-0.074, +0.141] | 1.000 | pass |

**Exclusions and analysis rules:**
- Dev tasks are excluded from every gate statistic, though they may be reported separately and
  labelled dev.
- No qual task is dropped or re-judged differently after results are seen.
- The single run per task per condition is the unit. Repeating a task at temperature 0 and seed
  42 would be pseudo-replication and is not used to raise n.

## 7. What must reproduce byte-for-byte, and what varies

**Must reproduce** (a second run that differs here is a harness bug):
- `tasks_digest`, `inputs_digest` and `protocol_sha256`;
- the catalog digest;
- each task's `requested_domains`;
- the round-1 condition-B selection receipt and its digest;
- the rendered tool lists for A and for round-1 B, with their bytes and digests;
- the system prompt digest;
- the fixture file digests;
- the stub and fault definitions;
- the judge verdict on a stored transcript.

On the same backend build and model digest, first-turn prompt tokens are expected to reproduce
(they did in 0.1). A difference there is reported, not ignored.

**Varies run to run:**
- model text and tool calls, and the rounds that follow from them;
- prompt and completion tokens after round 1;
- expansions, which depend on the transcript;
- latencies, wall-clock, request ids and timestamps.

## 8. Freezing

1. The corpus is frozen by `CORPUS-DIGEST.txt`:
   - `tasks_digest`: sha256 over `sha256sum` lines of the sorted `tasks/*.json`.
   - `inputs_digest`: the same over `domains.json`, `prompts/`, `schema/`, `fixtures/`, `stubs/`
     and `tasks/`.
   - `protocol_sha256`: the digest of this file.

   CI (`bench-structural`) recomputes all three on every push. To reproduce by hand:
   `cd bench && find tasks -type f | LC_ALL=C sort | xargs sha256sum | sha256sum`.
2. Before the first qualification request, the digests and the run manifest (section 2) are
   committed to `main`. The run records the digests it ran against. A run whose recorded digests
   differ from the frozen ones does not count.
3. Dev runs (pilot, harness debugging) may happen before or after the freeze and never touch
   qual tasks.

## 9. Raw evidence

Every request digest, response usage, decision, result status and judge verdict is kept under
`bench/runs/<run-id>/`. Tool arguments are stored only as digests and key names, as in the 0.1
receipt. Large raw outputs are kept out of git as in `qualification/evidence/raw/`.

## 10. Change log

- 2026-10-04: initial pre-registration (domain rule v1, 44 tasks, thresholds above).
