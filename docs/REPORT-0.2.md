# INTERPLANE 0.2 capability report (2026-10-04)

Repository: https://github.com/aien-dev/interplane. Format follows `docs/REPORT-0.1.md`. Every
number below comes from a file on main (path cited). Nothing planned is described as done.

**Headline: the pre-registered CrossAxis gate FAILED.** The thresholds were fixed in
`bench/PROTOCOL-0.2.md` section 6 before the run and are not changed here.

## Observed

- Pre-registered gate, run `qual-20261004T2207Z`, 37 qualification tasks, 37 complete pairs,
  model `qwen3.5:9b` on Ollama 0.34.0 (`bench/runs/qual-20261004T2207Z/summary.md`, `manifest.json`):

| criterion | value | threshold | result |
|---|---|---|---|
| T median tool-schema token reduction | 0.877 | >= 0.70 | PASS |
| S success not worse (A full catalog 0.676, B CrossAxis 0.730) | delta 0.054, Newcombe 95% CI [-0.085, 0.191], McNemar exact p 0.688 | CI lower bound >= -0.10 | PASS |
| O1 required tools exposed after expansion | 0.865 | >= 0.95 | FAIL |
| O2 recovered within one expansion | 0 of 5 | >= 0.90 | FAIL |

  Overall verdict: FAIL.
- Cause of the O1/O2 failure (verified from the run files). All five failures are the five
  `expansion-*` tasks, which are built so the first selection omits a needed tool. Condition B
  offered the read-only discovery tool `interplane_capabilities_search` on every round
  (`bench/tools/run_bench.py` line 239). The model did not use it on any of those five tasks.
  Across all 37 B runs the summary records exactly one discovery call (condition A: 0), and
  that call was in `unknown_tool-002` (receipt `receipts/unknown_tool-002.B.json`). So
  model-driven expansion did not happen. Reading: this model, with this prompt, does not
  discover tools on its own; this is model behaviour, not a harness fault (re-judge mismatches 0,
  first-turn measured vs real token mismatches 0, per the summary diagnostics).
- Both conditions failed all five `expansion` tasks (A 0/5, B 0/5), so those tasks also failed
  with the full catalog (check `answer_contains_all`). In B they additionally failed
  `expansion_occurred`.
- Noise in the data, stated plainly: execution errors were 51 of 137 calls in A and 36 of 120 in
  B, 87 of 257 in total (summary, "Condition A/B"). The tool backends are simulated, so many
  errors are by design (exec_failure tasks), but not all. Empty final answers: A 6, B 4 (the
  model's thinking consumed the budget). Wall-clock is affected by another inference process
  (`llama-server:34495`) seen during the run.
- Other measured effects (same summary): first-turn prompt tokens median 15 977 (A) vs 2 133 (B);
  median first-turn prompt-token reduction 0.866; median total-prompt-token reduction 0.868;
  median rounds 4 vs 4; unnecessary-call rate 0.358 (49 of 137) in A vs 0.200 (24 of 120) in B.
  Required tool missing on the first turn and at the end: A 0 of 37, B 5 of 37.
- Success by category: B was lower than A on `ambiguous` (1/3 vs 2/3) and `denied` (2/3 vs 3/3);
  higher on `injection_tool`, `rare`, `sequential`, `unknown_tool`. With 37 pairs (A only 2, B only
  4) these category differences are not conclusions.

## Backends

Qwen3.5-9B, all with the same 14-row probe, Rust and Python, reports under
`qualification/reports/`, captures and per-backend `NOTES.md` under `qualification/captures/`:

| backend | version and setup | Python probe | Rust probe |
|---|---|---|---|
| Ollama | 0.34.0 (0.1) | 13 PASS, 1 UNSUPPORTED (`tools.text_qwen35`) | same verdicts |
| llama.cpp | b11398 CUDA build, same GGUF blob as Ollama, official template | 14/14 PASS | 14/14 PASS after probe alignment (PR #6) |
| SGLang | 0.5.20, docker `lmsysorg/sglang:latest-cu130`, bf16, `--tool-call-parser qwen3_coder`, `--reasoning-parser qwen3` | 14/14 PASS | 14/14 PASS on the aligned Rust probe; the earlier 12/14 run is kept as `...probe.rust.pre-alignment.json` |

The 12/14 result was caused by the two probes sending different prompts, not by a backend fault;
`spec/PROBE.md` now requires identical request bodies. The aligned Rust probe was rerun on SGLang
by the orchestrator. (The SGLang `NOTES.md` still describes the pre-alignment state in its
"Probe results" section; the report files above are authoritative.) Single run per probe; no
repeat runs.

Behaviour differences found (from the `NOTES.md` files):

- Reasoning field: `reasoning` on Ollama, `reasoning_content` on llama.cpp and SGLang. Disabling
  thinking: `reasoning_effort: "none"` and `enable_thinking: false` work on llama.cpp and SGLang;
  `think: false` is ignored on all three OpenAI-compatible paths.
- Streaming: Ollama sends the tool call as one chunk; llama.cpp and SGLang fragment it across
  chunks (clients must accumulate by index).
- Malformed arguments: llama.cpp silently coerced a string to the integer 1 where the schema said
  integer (`m1b`, mechanism not investigated); SGLang rejects replayed invalid argument JSON with
  HTTP 400 (model-side malformed output was not elicited).
- Raw text form: llama.cpp and SGLang pass the Qwen3.5 XML through as content when no `tools` are
  sent and parse it when `tools` are sent. SGLang content begins with `"\n\n"` under the reasoning
  parser.
- Limits: only the `qwen3_coder` SGLang parser was tried; SGLang used bf16 weights, so engine
  and quantization effects mix there.

## AIEN authority

- aien-sovereign-core #203: `EffectAuthority` trait and `EffectClassAuthority` in `aien-mcp`; the
  `AuthorizedEffect` mint is crate-private. #204 (merge 6554aac): single-use approvals
  (`ApprovalDesk`, `ApprovalGrant`, `EffectLane::authorize_approved`); `EffectLane` is at
  `crates/aien-mcp/src/broker.rs:223`; replay returns the existing receipt through the
  idempotency ledger.
- interplane #3 and #12: the AIEN adapter runs on that real authority path, pinned to 6554aac, with
  six approval tests: once, `Consumed`, replay returns the existing receipt, `Mismatch`, `Expired`
  (adapter tests in `adapters/`; audit in `docs/audits/aien-authority-reaudit-2026-10-04.md`).
- Limits: grants live in memory only; a grant is spent at mint even if the provider then rejects
  the call; the desk is separated from other desks by handle discipline, not by type; the clock
  is supplied by the host; the interplane `Pipeline` has no grant field, so approvals reach the
  authority through the adapter, not through the generic pipeline.

## Changed

- CrossAxis: real token accounting (`selection.measure` units), bounded expansion and discovery in
  Rust and Python, conformance cases 24 and 25 (PR #4).
- Benchmark: 44 tasks (7 development, 37 qualification) over 13 categories, protocol with the
  gate fixed before the run, offline structural CI, paired runner and analyzer (PRs #5, #11).
- Qualification on llama.cpp and SGLang; probe alignment (PRs #6, #7).
- External evidence (below), upstream Odysseus #6474 branch check (`docs/upstream/odysseus-6474-pr.md`).

## Proven

- 0.1 conformance, run locally on this branch on 2026-10-04: Rust and Python conformance runners
  each report 25/25 PASS and the two verdict files are byte-identical (`cmp`). Rust `cargo test
  --all`: 92 passed, 0 failed.
- The paired benchmark ran to completion with 0 infra failures and 74 receipts in
  `bench/runs/qual-20261004T2207Z/receipts`.
- Selection cut exposed tool-schema cost by a median 87.7% on this catalog (71 tools) without a
  measurable drop in overall success (S passes), on this model.
- The approval path on the AIEN adapter consumes single-use grants (limits above).

## Failed

- Gate criteria O1 and O2 (above). The 0.2 claim "minimal exposure without loss of capability"
  is NOT established: exposure is minimal, but the safety net for a wrong selection did not work
  with this model and prompt.
- Both conditions did poorly on tasks with simulated tool failures and empty answers; the success
  rates (0.676, 0.730) are low in absolute terms.

## Unproven

- Whether expansion works when triggered by the runtime instead of the model (not tested).
- Any other model, catalog or prompt; the result is one model on one catalog, one run.
- Repeat-run stability of the benchmark (single run).
- Whether the 87 execution errors changed the S result (not analyzed).
- Wall-clock effects (another inference process was on the GPU).

## Gate table, 0.2 checklist (master plan section 15)

| item | result | evidence |
|---|---|---|
| Paired bench complete (30-50 tasks) | PASS | 37 qualification pairs plus 7 development tasks; `bench/runs/qual-20261004T2207Z` |
| Real token accounting | PASS | `tokens_model_reported`; token-count stability check stable (15977 vs 15977) |
| CrossAxis evidence | FAIL | T and S pass, O1 0.865 and O2 0/5 fail |
| llama.cpp qualified | PASS | b11398, 14/14 both probes |
| SGLang qualified | PASS | 0.5.20, 14/14 both probes |
| AIEN production authority real | met with limits | #203, #204, interplane #12; limits listed above |
| 0.1 conformance green | PASS | 25/25 both languages, identical (run on this branch) |
| Report | PASS | this file |

Overall 0.2 gate: **not met**, because of the CrossAxis row.

## External lanes (evidence only; no upstream action has been taken)

- NightDriverStrip #666: already fixed upstream (commit d73d18db). Evidence is a host build of the
  real `socketserver.cpp` with a Rust test harness; no device run (`docs/external/nightdriver/666/REPORT.md`).
- NightDriverStrip #565: model-only evidence; the root cause is not proven. A capability mismatch
  in buffer sizing was verified in code. A device run is needed to confirm
  (`docs/external/nightdriver/565/REPORT.md`).
- Jan #8975: reproduced; a fix is prepared on a fork and not submitted
  (`docs/external/jan/8975/REPORT.md`). Another contributor had offered a fix; the design is
  undecided upstream.
- Odysseus #6474: branch verified against current dev, no upstream PR opened.
- Upstream actions are pending owner review. No partnership or acceptance is claimed.

## Architectural impact

- None to the wire format. Authority stayed outside INTERPLANE: the adapter asks AIEN's
  `EffectAuthority`, and only AIEN code mints `AuthorizedEffect`.
- The failed gate is about who triggers expansion. The Core `expand` function is correct and
  conformant; what failed is relying on a 9B model to ask for it.

## Next step (future work, NOT results)

0.2.x would be a new pre-registered protocol with its own thresholds written before any run. The
0.2 verdict stays FAIL. Candidate designs:

1. Runtime-triggered expansion: the runtime expands on an unknown-tool call, a failed call, or an
   approval step, with no model action needed.
2. A stronger discovery prompt for the model-driven path.
3. Reasoning disabled (`reasoning_effort: "none"`) to remove the empty-answer failures, with
   simulated backends fixed so that execution errors come only from the designed failure tasks.

Then 0.3 Trust (provenance, untrusted-context semantics, authorization states).
