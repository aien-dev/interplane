# INTERPLANE 0.2y: pre-registration for the held-out tool-discovery fix

Status: **pre-registered, no run exists, no task exists yet.** Nothing below describes a result.
Written 2026-10-05 after the 0.2x campaign (`docs/REPORT-0.2x.md`, run
`bench/runs/heldout-0.2x-20261005T0102Z`). The 0.2x held-out set is **spent**: it was used here to
diagnose, so it is dev data from now on and never enters a 0.2y gate statistic. The 44 tasks of
0.2 are dev data too. Changing anything here after the first 0.2y gate run invalidates that run;
every change goes in section 11 with date and reason.

## 1. Diagnosis (from the 0.2x receipts; nothing re-run)

Target: the 8 `nopath` expansion tasks (`expansion-117` to `-124`), recovered 0 of 8 in all four
arms (`docs/REPORT-0.2x.md`, O2 per kind table). Source: `seed-42/`, `seed-43/` and `seed-44/` `receipts/expansion-11x.*.json`.

| question | answer | evidence | strength |
|---|---|---|---|
| Was a discovery tool offered? | Yes, in every B round | `exposed_tools_count = exposed_caps_count + 1` in 677 of 677 B rounds of the 24 expansion tasks, 3 seeds | PROVEN |
| Was it called? | Almost never on `nopath` | 0 calls in 72 B1, B2 and B4 runs (8 tasks x 3 seeds x 3 conditions); 3 calls in 24 B3 runs, all `expansion-122`, each with `discovery_hits: 0` | PROVEN |
| Did a runtime trigger fire? | No | `expansions: []` and no `runtime_triggers` entry on any of the 96 `nopath` B1 to B4 runs; T1 to T3 only watch unknown names, unexposed names or typed missing-capability errors | PROVEN |
| Why not? The selector exposed the wrong domain. | Each `nopath` request contains a keyword of a non-file domain, so the frozen rule (`bench/domains.json`) exposed that domain and no file tool | derived `requested_domains` and round-1 `exposed_names`: 117 "task" -> tasks (`manage_tasks`, `todowrite`, `update_plan`); 118 "meeting" and 120 "event" -> calendar; 119 "remember" -> memory; 121 "contact" -> contacts; 122 "email" -> email; 123 and 124 "model" -> models. Answers sit in workspace files (`allowed_alternatives` = read_file, grep, glob, ls; `required_capabilities` empty) | PROVEN |
| What did the model do? | Called an exposed wrong-domain tool, got `ok`, an error or an approval request, and stopped | e.g. 118 and 120: `manage_calendar` status `ok` in all four B arms, no further call; 117: `manage_tasks` `execution_error` then answer; 123 B1: `list_served_models`, `list_models`, `manage_endpoints`, `search_hf_models`, `chat_with_model`, `list_cached_models`, none yielding the answer | PROVEN (calls); what it said UNVERIFIED |
| Did discovery queries fail to match? | Where it was called on `nopath`, yes | `expansion-122` B3: `discovery_hits: 0` in all 3 seeds. Across all 24 expansion tasks 53 of 76 B discovery calls had 0 hits (B1 13 of 21, B2 21 of 29, B3 16 of 23, B4 3 of 3), because `discover` is an ASCII substring match on tool name or description (`python/interplane/crossaxis.py:456-473`) | PROVEN (hit counts); query text UNVERIFIED |
| Is it a harness bug or a config gate? | No evidence of one | tool spec present, call path works (`bench/tools/run_bench.py:575-600` produced hits on `named` and `path` tasks: 8, 8, 7 B1/B2/B3 calls with hits); `max_rounds`, caps and the 30 % budget never bound (`expansions: []`) | LIKELY |
| Does the full catalog solve these tasks? | Rarely | judge pass A 3 of 24, A4 6 of 24 (expansion-123 for A, 123 and 124 for A4 in seed 42); A2 3 of 24 | PROVEN |

Root cause: **the model had a plausible exposed tool in the wrong domain and no reason to look
for another, and every runtime trigger is blind to that case** (the 0.2x section 9 prediction,
confirmed). That discovery is never self-started on `nopath` tasks is PROVEN; why the 9B model does
not start it is UNVERIFIED, because receipts keep `text_digest` and `arguments_digest` only: no
model text and no query string was recorded. Secondary finding: the `nopath` tasks are hard even
for the full catalog (A pass 3 of 24), so they measure model initiative, not only exposure.

Not claimed: that any other model, backend or prompt behaves the same.

## 2. The fix under test (bounded)

One change, **F1: a fixed base set of workspace-read tools is always exposed in condition B.**
Observed need: the answer to a `nopath` question is a workspace fact, and the file tools are the
route the baseline uses (A passes 123 by `grep`).

- Code: `bench/tools/run_bench.py:72`, `ALWAYS_INCLUDE = ["ask_user"]` becomes the frozen list
  `["ask_user", "ls", "glob", "grep", "read_file"]` for the new condition only (about 5 lines
  including the condition id and manifest field). `select(... always_include=...)` already pins
  these names (`python/interplane/crossaxis.py:224-240`); no selector or adapter change.
- Cost estimate (UNVERIFIED until measured offline before freeze): 0.2x section 5 measured
  `read_file` added at about +0.008 and an 8-tool widening at about +0.077 of the 59024-byte
  catalog, so four tools are expected near +0.03 to +0.05. The T gate (median >= 0.70) and the
  30 % budget must still hold; the exact figure is measured on the frozen tasks before the first
  model request and recorded.
- Receipts (**R1**, harness only, no behaviour change): record the discovery query string and the
  full assistant text of every round in the receipt (today digests only), so the UNVERIFIED rows
  above can be closed next time. A run without them is invalid.
- Out of scope, not tested here: a T4 trigger on wrong-domain results, prompt changes, a larger
  model.

Honest limit stated now: F1 makes `nopath` coverage true by construction for file questions, so
coverage (O2) alone proves little. The primary metric below is task success, not coverage.

## 3. Hypothesis

H1 (primary): with F1, a narrowed catalog answers discovery-needed tasks the full catalog can
answer, which 0.2x condition B3 could not (B3: 0 of 24 judged passes on `nopath`, 3 seeds x 8 tasks).
H0: F1 does not raise success on those tasks over B3.
H2 (guard): F1 keeps tool-schema cost and overall success within the 0.2x gate margins.

## 4. Arms (Qwen3.5-9B, Ollama 0.34.0, Odysseus 2992bf6, as 0.2x; a pass covers only these)

| condition | catalog | discovery tool | triggers | base set |
|---|---|---|---|---|
| A | full 71 | no | none | n/a |
| B3 | Select + expansion | yes | runtime-v1 T1 to T3, unchanged | `ask_user` |
| B5 (fix) | as B3 | yes | runtime-v1, unchanged | `ask_user, ls, glob, grep, read_file` |

Everything else (system prompt, `temperature 0`, `max_tokens 4096`, `MAX_ROUNDS 8`, timeout, sim-1
backends, reasoning default, caps) is identical to 0.2x and frozen by digest. B3 is the control
that carries the 0.2x mechanism, so the fix effect is B5 against B3 on identical new tasks.

## 5. Fresh task set (leakage rules)

**Composition (120 tasks, all qual, none run before freeze):**
- 48 `discovery_needed` tasks (the target): the user states a need in everyday words, names no
  tool, no path, no file extension, and the answer exists only in the workspace. 24 are authored so
  their keywords land in a non-file domain (as 117 to 124 did), 24 in no domain (default
  filesystem), so the fix is tested where the selector is wrong and where it is right.
- 72 regression tasks: the 0.2x section 3 categories other than `expansion`, drawn in proportion to
  those counts; the exact per-category counts are written into `heldout-0.2y/AUTHORING.md`
  before authoring. They need no file tool beyond what the matching 0.2x category needs.

**Who writes them.** A task author from a different model family than Qwen and than the session
that wrote this document (Codex `gpt-6-sol` or Gemini; the choice and the exact brief are
recorded in `heldout-0.2y/AUTHORING.md` before authoring). The author receives only: the task
JSON schema, the category list with counts, the fixture-writing rules, and a request for
natural-language user requests. The author does **not** receive: the catalog or any tool name, the
0.2x or 0.2 tasks, the domain keyword list, this diagnosis, or the identity of F1. Fixtures
(workspaces, stores, stubs) are written by the same author for the new tasks, never reusing 0.2x
fixture content.

**Leakage prevention (enforced by a new `validate.py --corpus 0.2y` lint, stdlib only).** A task
is rejected if its `user_request` contains any of: a catalog capability name, any token of one
(split on `_ . -`, length >= 3, the lint uses the recorded 71-name catalog) **that is not itself a
`domains.json` keyword**, the words `file`,
`path`, `grep`, `search tool`, `tool`, `function`, `command` (these stay banned even where they are
keywords), an extension from
`domains.json` `filename_pattern` plus `.rst .adoc .org .tsv .conf .xml .html`, a path separator,
or a name that equals a fixture file name. The lint also records the `domain_match` derivation per
task and fails the corpus if fewer than 24 `discovery_needed` tasks derive a non-file domain, or if
any discovery-needed task derives the filesystem domain with a keyword other than the default.
Task authors are told only the lint's failure messages, never the word lists, until freeze.

Why the keyword exemption (measured 2026-10-05, before any task exists): 34 of the 94 non-file
keywords in `domains.json` are also name tokens of the 71-name catalog (for example `email`,
`notes`, `models`, `calendar`, `tasks`, `plan`, `chat`), and the only `research` keyword is one, so
without the exemption the lint would reject most wrong-domain requests the design requires. A
keyword is the everyday word a user says ("check my email"), which is exactly how 117 to 124
misled the selector; a full capability name or a non-keyword name token (`manage`, `list`,
`served`...) would still name a tool and stays banned.

**How the 24 wrong-domain and 24 default tasks are obtained (no content-based selection).**
- The brief asks for 36 `discovery_needed` requests "phrased the way someone talks about their
  everyday work: mail, meetings, contacts, to-dos, notes, chats, models, memory, documents,
  images" and 36 phrased "as a plain question about something in their project". The brief
  names these life areas only, never a keyword list, a tool or the selector.
- The lint runs on every request in task-id order. A request is accepted into the non-file group
  if it passes the leakage rules and derives exactly one non-file domain, into the default group if
  it passes and derives the default list, and is otherwise discarded with its lint message
  recorded. Each group takes the first 24 accepted in id order; the rest are discarded unrun.
- If a group has fewer than 24 after one pass, the author gets one top-up request: "write k more
  requests of the same kind" plus the lint messages of the discarded ones (no word list), and the
  same rule runs on the new ids. At most three top-ups; if a group is still short, the corpus is not
  frozen, the shortfall is reported, and the authoring rules are revised under section 11 before a
  fresh authoring round.
- Every request ever written, accepted or discarded, and its lint message is kept in
  `heldout-0.2y/authoring-log.jsonl`. No person or model picks tasks by reading their content.

**Solvability calibration (before freeze, on a separate 30-task calibration set).** The same
author writes 30 extra `discovery_needed` tasks on different fixtures. Condition A is run on them
once, seed 42. The target-set authoring rules stay as written if A passes at least 40 % of the 30;
if not, the authoring rules (not the target tasks, and not B arms) are revised and the calibration
repeats with a fresh 30. No B condition runs on calibration or on the target set before freeze.
No target task is edited after any run touching it exists.

**Freeze.** `heldout-0.2y/CORPUS-DIGEST.txt` holds `tasks_digest`, `inputs_digest` and
`protocol_sha256` of this file, and is committed and merged **before** any B run starts, with the
manifest commit merged first (0.2x pushed manifests 13 s before its first request and merged two
minutes after; that order is not repeated). CI recomputes the digests.

## 6. Metrics and pass thresholds (seed 42 decides; evaluated on the 120 new tasks only)

Primary, **P**: judged task success on the 48 `discovery_needed` tasks, B5 against B3, paired.
- P passes iff exact McNemar one-sided p < 0.025 with B5-only passes greater than B3-only passes
  (`bench/tools/stats.py`), **and** B5 success >= 0.75 times A success on the same 48 tasks (so
  the gain is near what the full catalog gives, not only above a failing control).

Guards (all must hold, otherwise the fix fails even if P holds):
- **T:** median over the 120 tasks of 1 - (B5 per-round mean tool-schema tokens) / (A's) >= 0.70
  (0.2x definition).
- **S:** B5 against A over all 120 pairs, Newcombe method 10 95 % CI lower bound >= -0.10 and not
  (McNemar p < 0.05 with A-only > B5-only) (0.2x definition).
- **Safety:** zero denied or pending calls executed; zero expansions fired by one (N1, N2 unchanged).
- **Validity:** every round records R1 fields; no run with an unrecorded query or text counts.

Descriptive, not gated: O1/O2 coverage on the 48 (expected high by construction), per domain
subgroup (24 wrong-domain, 24 default), per template, B5 against B3 on regression tasks, trigger
counts, tool calls per run, and the share of B5 `discovery_needed` runs that called any file tool.

Pass overall = P and T and S and Safety and Validity. Anything else is FAIL, reported as such.

## 7. Sample size

n = 48 target pairs. If the calibration holds (A passes about 40 %) and B5 matches A while B3
stays near 0 (0 of 24 in 0.2x), the expected discordant split is about 19 to 0, far inside p < 0.025.
The smallest decisive result is 6 wins against 0 (exact one-sided p = 0.0156), 7 against 1
(p = 0.035, not enough). So P can only fail on a small effect or a failed fix, which is the
question. S uses n = 120, 0.2x section 7 power (about 0.75 at discordance 0.16, 0.85 at 0.12): an
S fail near the margin is reportable, not hidden.

## 8. Repeats, order, machine

Seeds 42 (decides), 43 and 44 (stability; a verdict flip is labelled **fragile**). Conditions per
task rotate (A, B3, B5) left by task index mod 3. One untimed dev run warms the backend per
seed. 120 x 3 conditions x 3 seeds = 1080 runs, about 3.3 h at the 0.2x median of 10.8 s per run.
Latency is reported only when `nvidia-smi` and `pgrep` show no other inference process before and
after each seed block (0.2x section 8); success and token figures stay valid otherwise. The run
holds the Spark quiet flag through `quietlock`.

## 9. Failed requests (new rule; 0.2x had none)

A run whose request fails with HTTP 5xx or a timeout is re-run once. If it fails again, the pair is
**excluded from P and S** and listed by id; the analysis reports P and S twice more as sensitivity
checks (counting the pair as B5 failure and as A failure). The verdict uses the exclusion rule;
if either sensitivity check flips the verdict the result is labelled **fragile**. More than 5 % of
runs failing twice in a seed block makes that block invalid (see stop rule). Known cause in 0.2x:
Ollama 0.34.0 returned 500 on malformed `qwen3.5` tool calls (`docs/REPORT-0.2x.md`).

## 10. Stop rule and receipts

- No interim analysis and no early stop on results. Each seed block runs to the end.
- Stop and mark the campaign **invalid** (not failed) only on integrity events: a digest differs
  from the frozen one; the quiet flag is lost; more than 5 % double-failed runs in a block; R1
  fields missing; a task found to leak a forbidden token after freeze (then all of it is
  discarded and a new set is pre-registered, never patched).
- Receipts per run use the 0.2x format plus R1 (round text and discovery query). Per seed:
  `manifest.json` (commit, digests, model digest, Ollama version), `gpu_at_start.txt`,
  `gpu_at_end.txt`, `receipts/`, and one `analysis/` produced by `bench/tools/analyze.py`
  extended for P (reviewed offline before freeze, no model needed). The analysis is re-run by a
  second person or session from the committed receipts before the report is merged.
- Report states every threshold unchanged, PASS or FAIL per criterion, the fragile flag and what
  stays UNVERIFIED.

## 11. Change log

- 2026-10-05: initial pre-registration, docs only. The fix is not implemented, no task is written,
  no run exists. Items still to be built before freeze: condition B5 and R1 in the runner,
  `validate.py --corpus 0.2y`, `AUTHORING.md`, the 30-task calibration set and its A run, the
  extended analyzer for P, and the frozen `CORPUS-DIGEST.txt`.
- 2026-10-05 (review, before any task or run exists): the leakage lint exempts name tokens that are
  also `domains.json` keywords (34 of 94 collide; without it the 24 wrong-domain tasks could not
  pass), and section 5 now fixes how the 24 wrong-domain and 24 default tasks are authored,
  accepted in id order, topped up and logged. Diagnosis source paths corrected to `seed-42/` to
  `seed-44/`. Thresholds, arms and sample size unchanged.
