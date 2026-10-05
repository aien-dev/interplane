# INTERPLANE 0.2x CrossAxis benchmark: pre-registered protocol (future work)

Status: **pre-registered 2026-10-04, before any model run on the held-out set. No results exist.**
Everything below describes a campaign that has not been run. Changing anything below after the
first held-out request invalidates that campaign. Every change is listed in section 13 with its
date and reason. The digest of this file is `protocol_sha256` in `heldout-0.2x/CORPUS-DIGEST.txt`.

This protocol extends `PROTOCOL-0.2.md`. Where this file is silent, sections 3 to 5 and 7 to 9 of
`PROTOCOL-0.2.md` apply unchanged (run lifecycle, judge, measures, reproducibility, raw evidence).
The 0.2 run `bench/runs/qual-20261004T2207Z` and its report stay as published; nothing here
re-judges them.

## 1. Question

0.2 failed its gate on O1 and O2: the model never used the discovery tool on the five expansion
tasks (`docs/REPORT-0.2.md`, lines 18 to 33). 0.2x asks, one factor at a time, what changes that:

1. model-driven discovery as in 0.2 (arm 1, the 0.2 mechanism on a fresh set);
2. a stronger, tool-agnostic discovery instruction in the prompt (arm 2);
3. expansion triggered by the runtime instead of the model (arm 3, **primary**);
4. reasoning disabled in the model (arm 4).

A pass covers only the model, backend and machine named in the run manifest.

## 2. Corpora

**Dev set (regression only).** All 44 tasks of the published 0.2 corpus (`bench/tasks/`,
`tasks_digest sha256:818ad12d42ba24c0e505f61e400ac9684dbfba68e56ee5c7e2ca5b7fb3e59e34`), dev and
qual alike, run with simulated backends `sim-1` (`bench/stubs/backends.json`). They were seen
before this protocol was written, so they never enter a gate statistic. They serve for harness
debugging, the pilot, the warm-up and regression checks, and are reported separately, labelled
dev.

**Held-out qualification set.** 120 new tasks in `bench/heldout-0.2x/tasks/`, authored for this
protocol on 2026-10-04 and never shown to any model. They use four new fixtures
(`heldout-0.2x/fixtures/{beacon,orchard,quarry,vault}`), their own private-data stores
(`heldout-0.2x/stores/`) and their own tool stubs (`heldout-0.2x/stubs/`). Fixture and stub paths
in these tasks are relative to `bench/heldout-0.2x/`; `system_prompt_ref.path` stays relative to `bench/`. The system prompt is the frozen 0.2 prompt
(`prompts/system.md`, sha256 `207dd449...aaf647f5`), and domains come from the frozen 0.2 rule
(`domains.json` v1). No held-out task is run, even once, before the freeze in section 12.

## 3. Held-out composition (enforced by `validate.py --corpus 0.2x`)

| category | tasks | | category | tasks |
|---|---|---|---|---|
| filesystem | 12 | | approval | 8 |
| ambiguous | 8 | | injection_workspace | 6 |
| multidomain | 10 | | injection_tool | 6 |
| rare | 8 | | expansion | 24 |
| wrong_first_tool | 8 | | unknown_tool | 6 |
| exec_failure | 8 | | sequential | 8 |
| denied | 8 | | **total** | **120** (all qual) |

The 24 expansion tasks are uncovered by the first condition-B selection by construction (CI checks
it). They come in three kinds of 8, tagged `expansion_kind` in each task's `notes`:

- `named`: the user names the excluded tool ("Use read_file on docs/keeper-rota.rst ...") and the
  request carries a non-file domain word. This is the fair test of the unexposed-tool trigger.
- `path`: the user gives a path whose extension the domain rule does not treat as a file name
  (`.rst`, `.adoc`, `.org`, `.tsv`, `.conf`).
- `nopath`: no path and no tool named, as in the five 0.2 expansion tasks.

Every task carries `template: <name>` in `notes`. 112 templates cover 120 tasks; 8 expansion
templates are used twice, once with a path or tool name and once without. Results are reported per
template as well as per task, so a shared template cannot hide as two independent results.

Every tool any task lists has a deterministic backend: a stub, a `sim-1` simulator, the reference
file tools, or a declared failure (bash and python). Expansion answers are absent from every
private-data store of their fixture (CI checks it), so they can only be read from the workspace.

## 4. Conditions and the comparison matrix

Seven conditions per task. Each arm changes **one** factor against a matched full-catalog
baseline that shares the arm's model, prompt and generation settings:

| condition | tools rendered | discovery tool | prompt | reasoning | runtime triggers |
|---|---|---|---|---|---|
| A (baseline for B1, B3) | full catalog, 71 | no | system.md | default | none |
| A2 (baseline for B2) | full catalog | no | system.md + addendum | default | none |
| A4 (baseline for B4) | full catalog | no | system.md | **off** | none |
| B1 (arm 1) | Select + bounded expansion | yes | system.md | default | T2 only, as in 0.2 |
| B2 (arm 2) | as B1 | yes | system.md + **addendum** | default | T2 only |
| B3 (arm 3) | as B1 | yes | system.md | default | **T1, T2, T3** (section 5) |
| B4 (arm 4) | as B1 | yes | system.md | **off** | T2 only |

- Each gate pair (A, B1), (A2, B2), (A, B3), (A4, B4) differs only in the exposure policy, as in 0.2.
  So each arm keeps the 0.2 question ("does Select lose capability?") under its own setting.
- Secondary comparisons B2 vs B1, B3 vs B1 and B4 vs B1 isolate the arm's factor within condition
  B. They are reported, not gated.
- **Addendum** (arm 2): `heldout-0.2x/prompts/discovery-addendum.md`, appended to `system.md`
  after one blank line. It names no tool, so A2 can share it.
- **Reasoning off** (arm 4): the request body adds `"reasoning_effort": "none"`. Ollama 0.34.0
  accepted it in the probe, with no reasoning in the reply
  (`qualification/reports/qwen35-9b-ollama-0.34.0.probe.json:104`). Every round of an A4 or B4
  run must record `reasoning_present: false`. A run with reasoning present is invalid for arm 4.
  For SGLang or llama.cpp the field is `chat_template_kwargs.enable_thinking = false`. The
  manifest records which field was sent.
- **Identical in all seven conditions:** model and digest, backend and version, `temperature = 0`,
  `max_tokens = 4096`, timeout 300 s, `MAX_ROUNDS = 8`, Odysseus `2992bf6`, catalog, fixtures,
  stores, stubs, fault injections, authority profiles and `sim-1` backends.
- **Unknown-tool judging** is unchanged from 0.2: a task passes if the answer reports the tool as
  unavailable **or** answers through a fallback (an `any_of` check). The two halves are reported
  separately, so a reader can see which one carried each pass.
- **Judge conditions:** checks marked `"conditions": ["B"]` apply to B1 to B4, and checks marked
  `["A"]` apply to A, A2 and A4.

## 5. Runtime-triggered expansion (arm 3)

The policy runs host-side in the bench runner and uses only the CrossAxis primitives that exist
today: `discover` and `expand` (`python/interplane/crossaxis.py:456` and `:476`, with the same
contract at `rust/crates/interplane-crossaxis/src/lib.rs:473` and `:507`). It looks only at events the pipeline
already records for each call: the `error_code`, the `capability`, the `exposed` flag and the
`status`.

**Triggers.** Each trigger is evaluated after Odysseus has decided the call:

| id | observable event | action |
|---|---|---|
| T1 unknown tool | `error_code = unknown_capability` (name not in the mapping table), or `capability_not_found` for a name not in the catalog | catalog search: `discover(catalog, selection, token)` for each token of the requested name split on `_ . -` with length ≥ 3. Hits are united, sorted and cut at 8, then `expand(..., {"kind": "discovery_hit", "query": <name>, "names": hits})` |
| T2 unexposed tool | a catalog capability called while not in the current selection (`exposed = false`), unless the call was denied or held for approval (N1, N2) | `expand(..., {"kind": "requested_excluded", "name": cap})` (already in 0.2, `bench/tools/run_bench.py:400-409` at commit 20ed1f4, under the comment `# ---- expansion and discovery, in call order`) |
| T3 typed missing capability | `error_code` is `capability_not_found` or `stale_capability` for a name that **is** in the catalog | widen the domain: `expand(..., {"kind": "requested_excluded", "name": cap, "include_domain_siblings": true})` |

For an unexposed catalog tool (T2), the catalog search the plan asks for reduces to an exact-name
lookup, which is what `requested_excluded` performs, so no substring `discover` call is needed.
T1 is the case where the search is not exact.

**Never triggers** (negative rules, each checked by a negative control):

- N1. `policy_denied` (status `denied`): stays denied. No expansion, no retry, no other tool
  offered in its place.
- N2. `approval_required` (status `requires_approval`): stays pending. The host creates no
  approval, consumes none and does not continue the call.
- N3. Exposure never grants permission. A newly exposed capability is decided by Odysseus like
  any other call.
- N4. Exposure never re-executes or retries the original request, including writes. The call that
  fired the trigger already went through the pipeline once, and its result is what the model sees.
  The host never issues a call on the model's behalf.
- N5. `invalid_arguments`, `execution_error` and `execution_timeout` are not missing-capability
  failures and never fire T3. They are the model's or the tool's problem. An unexposed call that
  fails this way still fires T2, because the model asked for a tool it could not see.

In B1, B2 and B4, T2 keeps its exact 0.2 behaviour, which also exposes a capability after a denied
or pending call (the call itself stays denied or pending). N1 and N2 as trigger rules apply to
B3, so B1 stays the 0.2 mechanism unchanged.

**Caps.**

- At most **2** effective expansions per run (`max_expansions`) and at most **8** capabilities
  added per expansion (`max_added_per_expansion`). These are the CrossAxis bounds recorded in
  `selector.expansion`, the same as 0.2.
- Exposed schema budget: the rendered tool list of any round stays within **30 %** of the
  full-catalog rendered bytes of the same task (the A round-1 `tools_bytes`). An expansion whose
  result would exceed the budget is not applied, and every name it would have added is recorded as
  refused with reason `schema_budget`. 30 % is the T threshold restated as a hard
  cap, so arm 3 cannot pass O1 by exposing everything. The cap can be met on every expansion task.
  Measured offline with `openai_tools_renderer` against the 59024-byte full catalog, the worst
  case over the 24 held-out and 5 dev expansion tasks is:
  - round 1: 0.179;
  - round 1 plus `read_file` (T2): 0.187;
  - round 1 plus a full `read_file` domain widening (T3, 8 added): 0.256.
- Rounds: `MAX_ROUNDS = 8`, unchanged.
- Retries: each (trigger, name) pair is acted on at most once per run. A repeat is logged in
  `runtime_triggers` as `duplicate_trigger` and adds nothing.
- When a trigger adds nothing because a cap refused its candidates (`max_expansions`,
  `max_added_per_expansion` or `schema_budget`), the host appends one fixed note to that call's tool result: "No further
  tools can be made available in this conversation. Answer with the tools you have, or tell the
  user what is missing." The model may then state the limitation or ask the user (`ask_user` is
  always exposed). The host takes no other action.

**Receipts.** Every trigger decision is logged in the run's `runtime_triggers`, including calls
suppressed by N1 or N2, duplicates and searches with no hits. Every attempted expansion also
appears in `expansions`, which the judge and the 0.2 metrics read. An expansion event carries:
`trigger` (T1, T2 or T3), the call index and round, the triggering `error_code` or `exposed` flag,
the previous selection digest and exposed names, the new selection digest and added names, the
refused names with reasons, the remaining budget (expansions left, bytes left), and a `reason`
string. The selection receipt itself is the one `expand` returns (`selection_digest`,
`parent_digest`, `expansions` log), so Python and Rust produce byte-identical selection receipts
for the same inputs.

## 6. The gate (decided now; evaluated on held-out tasks only, seed 42)

Each arm k in {1, 2, 3, 4} has its own gate against its matched baseline. **Arm 3 is the primary
claim.** A pass in arms 1, 2 or 4 is reported as exploratory and becomes a claim only after a fresh
held-out run.

- **T, tool-schema cost.** For each task, r = 1 - (per-round mean tool-schema tokens in Bk) /
  (the same in its baseline). The criterion holds if **median(r) ≥ 0.70**. Tokens are measured as
  in 0.2. If the backend reports no token usage, the run is invalid for T.
- **S, success not worse.** Paired counts (e, f, g, h) over the 120 pairs. Same method as 0.2:
  Newcombe method 10, 95 % CI, exact McNemar (`bench/tools/stats.py`). **S holds iff the CI lower
  bound ≥ -0.10 AND NOT (McNemar p < 0.05 with g > f).**
- **O1, required tools exposed.** Among tasks with requirements, Bk's final exposed set covers
  them in **≥ 95 %** of tasks, so at most 6 of 120 may end uncovered. All non-expansion tasks are
  covered on round 1 by construction, so O1 rests on the 24 expansion tasks.
- **O2, recovery by expansion.** Among the 24 expansion tasks, coverage is reached within
  **1 effective expansion** in **≥ 90 %**, which means **≥ 22 of 24**. O2 is the binding
  omission criterion.

Two diagnostic measures are reported for every arm next to O2. They are pre-registered here and
do not gate:

- **O2a, trigger coverage.** The number of the 24 expansion tasks on which any runtime trigger
  (T1, T2, T3) or a model discovery call fired at least once.
- **O2b, recovery given a trigger.** Among the O2a tasks, the number whose coverage was reached
  within 1 effective expansion.

O2 itself stays the gate as written above. The product claim is that a narrowed catalog loses
nothing a task needs, and O2 measures that outcome directly. O2a and O2b only explain a pass
or a fail: a low O2a means the mechanism had no signal to act on (section 9), a low O2b means
it acted and still did not recover.

An arm passes only if all four hold. O1 and O2 are also reported per expansion kind (named, path,
nopath). A kind-level figure is descriptive and does not gate.

**Exclusions.** No held-out task is dropped or re-judged after results are seen. Dev tasks never
enter a gate. Runs with an infrastructure failure are re-run once. If the re-run also fails, the
pair counts as a failure for the B side.

## 7. Sample size

`python3 bench/tools/stats.py --power` gives the share of simulated campaigns in which S holds
(seed 1, 3000 trials per cell, both-succeed rate 0.75). pd is the discordant-pair rate.

| n pairs | pd 0.08 | pd 0.12 | pd 0.16 | pd 0.20 | pass rate at the margin θ = -0.10 (pd 0.12 to 0.20) |
|---|---|---|---|---|---|
| 60 | 0.687 | 0.530 | 0.427 | 0.358 | 0.019 to 0.023 |
| 100 | 0.898 | 0.771 | 0.659 | 0.561 | 0.023 |
| **120** | **0.946** | **0.846** | **0.747** | **0.654** | 0.019 to 0.025 |
| 150 | 0.978 | 0.917 | 0.838 | 0.758 | 0.016 to 0.029 |

- The 0.2 qual run had 6 discordant pairs in 37, pd ≈ 0.16.
- At n = 120 a truly equal arm passes S with probability about 0.75 at that discordance, and 0.85
  at pd 0.12.
- An arm exactly at the margin (θ = -0.10, the boundary of the null hypothesis) passes at most about
  2.5 % of the time. That is the size of the test, as intended for a one-sided 95 % bound.
- 150 pairs would add about 0.09 power at pd 0.16 for 25 % more authoring and run time. 120 was
  chosen as the budget.
- O2 at 24 tasks: 22 of 24 is the smallest count meeting 90 %. A true recovery rate of 0.95 passes
  with probability 0.88, 0.90 with 0.56 and 0.85 with 0.28 (exact binomial, stated before any run).

## 8. Repeats, order and the machine

- **Repeats:** every condition runs three times, at `temperature = 0` with seeds **42, 43 and 44**.
  The gate verdict uses seed 42 only. Seeds 43 and 44 are stability checks. If any gate criterion of
  an arm would flip under seed 43 or 44, the arm's verdict is labelled **fragile** in the report.
  Repeats never raise n.
- **Order:** tasks in id order. Within a task, the seven conditions run in the order A, A2, A4,
  B1, B2, B3, B4, rotated left by (task index mod 7). One untimed dev-task run warms the backend
  before each seed block.
- **Uncontended accelerator for latency.**
  - Before each seed block and after the last run, `nvidia-smi` lists the compute processes
    (`gpu_at_start.txt`, `gpu_at_end.txt`), and `pgrep` finds no other inference or training
    process (`ollama runner` beyond the serving one, `llama-server`, `sglang`, `vllm`, training
    jobs).
  - If either check fails, that block's latency figures are labelled invalid. Its success and
    token figures stay valid.
  - Latency is reported, not gated.

## 9. Prediction from 0.2 data (stated before the run)

- In the 0.2 qual B receipts, T2 fired twice (denied-003, injection_tool-002), neither on an
  expansion task. T1 and T3 would have fired zero times: there were 0 `unknown_capability` and 0
  `capability_not_found` results in condition B.
- On the five 0.2 expansion tasks, the model called exposed wrong-domain tools
  (`manage_notes`, `list_models` and others), which no trigger covers.
- So for `nopath` tasks, arm 3 is predicted to behave like arm 1, and O2 is predicted to fail
  there. The `named` kind is where T2 can fire.
- This prediction is recorded so a pass or fail cannot be explained after the fact. It is not an
  exclusion rule.

## 10. Campaign preparation (required before the first held-out request; not yet built)

- Runner: `--corpus 0.2x` (task root `bench/heldout-0.2x`); `--condition` for the seven
  conditions; the prompt addendum; the reasoning-off request field, since today `Client.chat`
  sends only model, messages, temperature, seed, max_tokens and tools (`run_bench.py:134-143` at commit 20ed1f4);
  `--backends sim-1` required for both corpora.
- Arm 3 policy as in section 5, with negative controls: on denied and approval tasks, zero
  unauthorized executions and zero expansion events fired by a denied or pending call.
- A pilot on dev tasks only, with all seven conditions.
- **Cost:** 120 tasks × 7 conditions × 3 seeds = 2520 runs. At the 0.2 median of 10.8 s per run
  (`qual-20261004T2207Z`, n = 74) that is about 7.6 h, plus the `max_tokens = 1` measurement
  requests and the dev runs (44 × 7 = 308 runs, about 0.9 h). Reasoning-off runs are expected to
  be shorter.

## 11. Reporting

For each arm: T, S (with the paired table), O1 and O2 as in section 6. Also:

- per-category success;
- O1 and O2 per expansion kind;
- results per template;
- trigger counts by id;
- refusals by reason, including `schema_budget`, and `duplicate_trigger` and suppressed (N1, N2) counts;
- cap-exhausted notes;
- the fragile flag;
- latency validity.

Secondary comparisons (B2, B3 and B4 against B1) are reported as success counts and coverage.
Negative controls are reported as counts that must be zero.

## 12. Freezing

`heldout-0.2x/CORPUS-DIGEST.txt` freezes:

- `tasks_digest` over `heldout-0.2x/tasks/*.json`;
- `inputs_digest` over `domains.json`, `prompts/`, `schema/`, `stubs/backends.json`,
  `tools/sim_backends.py` and `heldout-0.2x/{fixtures,prompts,stores,stubs,tasks}/`;
- `protocol_sha256` of this file.

CI (`bench-structural`) recomputes all three on every push with
`python bench/tools/validate.py --corpus 0.2x --require-jsonschema`. A campaign whose recorded
digests differ from the frozen ones does not count.

## 13. Change log

- 2026-10-04: initial pre-registration (120 held-out tasks, seven conditions, thresholds above).
  No held-out run exists.
- 2026-10-04: runner line citations pinned to commit 2de4c81 so later runner edits cannot move
  them. No held-out run exists.
- 2026-10-04: runner citations re-pinned from 2de4c81 to 20ed1f4, the same runner content as
  merged to main (#21 squash). Added the non-gating diagnostics O2a and O2b (section 6) after
  checking the 0.2 qual receipts: condition B produced 0 `unknown_capability` and 0
  `capability_not_found` results, so a gate change toward what arm 3 can reach was rejected;
  O1 and O2 are unchanged. No held-out run exists.
