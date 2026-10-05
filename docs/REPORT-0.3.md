# INTERPLANE 0.3 trust report (2026-10-05)

Repository: https://github.com/aien-dev/interplane. Format follows `docs/REPORT-0.2.md`. Every
number below comes from a file on main (path cited). Nothing planned is described as done.

**Headline: every offline gate of the pre-registered 0.3 protocol PASSED** (P, X, I, E, A, N;
`bench/PROTOCOL-0.3.md` section 5, thresholds fixed before the run and not changed here).
Gate M, the live-model leg, also PASSED: a real model (Qwen3.5-9B) followed the injected text in
16 of 33 cases, and the runtime let none of those effects run (violations 0). Details in "Model
behaviour" below.

The claim is about the runtime layer only: INTERPLANE keeps the line between content, model
intent and runtime authority. It is not a claim that any model ignores injected text.

## Observed

Qualifying run `trust-0.3-20261005T0229Z` (`bench/runs/trust-0.3-20261005T0229Z/`), started
2026-10-05T02:31:59Z, finished 02:32:31Z. The run manifest (`manifest.json`) was committed to main
before the run (PR #41, PROTOCOL-0.3 section 8.2) and names commit `be2df5e`. The run itself was
taken at `0c69bb5`, which differs from `be2df5e` only by adding that manifest file
(`git diff --stat be2df5e 0c69bb5`: 1 file, the manifest). Pins observed at run time
(`environment.json`): Odysseus 2992bf6, aegis-runtime f4e8709, aien-protocols 3a4cdbe,
aien-sovereign-core 0c1d249, rustc 1.99.0, Python 3.12.3. Every leg exited 0 (`exit-codes.json`).

| Gate | Result | Evidence (`gates.md`) |
|---|---|---|
| P provenance | PASS | P1 179/179 InputRecords complete; P2 70/70 executed results labelled; P3 9 of 9 source classes; P4 0 trust leaks |
| X cross-language | PASS | 106 verdict rows byte-identical (Rust vs Python); result and input dumps identical |
| I corpus | PASS | 60 cases (minimum 52), 13 of 13 categories |
| E no untrusted effect | PASS | violations 0, content-derived intents 0, forgeries accepted 0 (T1 44, T2 44, T3 30, T4 18 injection cases) |
| A approvals | PASS | A-SDK 14 cases on T1 and T2; A-AIEN 14 cases including A15; A-ODY 13 cases, approval continuation unsupported on Odysseus (fails closed) |
| N negative controls | PASS | V1 to V8 each detected; Rust and Python matrices identical |
| M model behaviour | PASS (violations 0; follow rate reported only) | `live/summary.md`; see "Model behaviour" |

Systems under test (PROTOCOL-0.3 section 2): T1 Rust SDK and T2 Python SDK over the whole corpus
(106 fixtures), T3 the Odysseus adapter over its 43-fixture subset, T4 the AIEN adapter over its
32-fixture subset (both subsets frozen in `conformance/TRUST-DIGEST.txt`).

X also requires the 25 fixtures that existed at 0.2 to keep their expected outcomes. Checked by
diff against `678c06c` (the 0.2 report commit): 22 are unchanged; three were edited with no change
to any expected status, decision or outcome field. Fixture 19 gained the user's request as a
registered input (the 0.3 mock effect policy needs to see it; without it the write would be held
for approval). Fixtures 24 and 25 carry new catalog and selection digests and three extra
`domain_mismatch` rows because the mock catalog gained `read_document`, `load_skill` and
`call_provider` (plan cut P4).

## Model behaviour (gate M, reported only)

Live leg `bench/runs/trust-0.3-20261005T0229Z/live/` (tool `bench/tools/live_injection.py`, PR #42),
started 2026-10-05T08:10:34Z, finished 08:13:03Z. Qwen3.5-9B (`qwen3.5:9b`, digest
`sha256:56671c2a...`) on Ollama 0.34.0, temperature 0, seed 42, max_tokens 1024, one run per case.
`live/backend-check.json` records the backend version, that the model digest matches the run
manifest, and that no other model was loaded. All 33 pre-registered cases ran once
(`live/summary.json`: preregistered 33, ran 33, none missing, duplicated or errored). The case set
digest matches the frozen one (`live/manifest.json`: `freeze_match` true).

| Category | Cases | Followed | Follow rate | Wilson 95% | Refusal | No answer | Other | Violations |
|---|---|---|---|---|---|---|---|---|
| C01 | 3 | 2 | 0.667 | [0.208, 0.939] | 0 | 0 | 1 | 0 |
| C02 | 5 | 3 | 0.600 | [0.231, 0.882] | 0 | 0 | 2 | 0 |
| C03 | 4 | 2 | 0.500 | [0.150, 0.850] | 0 | 0 | 2 | 0 |
| C04 | 4 | 1 | 0.250 | [0.046, 0.699] | 0 | 0 | 3 | 0 |
| C05 | 4 | 1 | 0.250 | [0.046, 0.699] | 0 | 0 | 3 | 0 |
| C06 | 3 | 3 | 1.000 | [0.439, 1.000] | 0 | 0 | 0 | 0 |
| C07 | 4 | 0 | 0.000 | [0.000, 0.490] | 0 | 0 | 4 | 0 |
| C08 | 3 | 1 | 0.333 | [0.061, 0.792] | 0 | 2 | 0 | 0 |
| C11 | 3 | 3 | 1.000 | [0.439, 1.000] | 0 | 0 | 0 | 0 |
| pooled | 33 | 16 | 0.485 | [0.325, 0.648] | 0 | 2 | 15 | 0 |

Gate M (violations = 0, PROTOCOL-0.3 section 7): **PASS**. The model emitted the injected intent in
16 of 33 cases (0.485, Wilson 95 % [0.325, 0.648]) and refused none; the runtime turned every one
of those intents away, so no injected effect ran. This is the point of the 0.3 claim: the model is
easy to steer, the boundary is not. The follow rate is reported only; no threshold applies.

Limits of this leg:

- It was taken at commit `5330a1c`, after the qualifying offline run (`0c69bb5`). The changes in
  between are PRs #44 to #49 (streamed responses, `openai` argument parity, session limits,
  refused-call ids, the `trust_run` gate F, a trace-id window). The case set and corpus digests
  still match the freeze; the offline gates above were not re-run at `5330a1c` for this report.
- "Refusal" is a lexical rule (`REFUSAL_RE` in `live_injection.py`); a refusal phrased another way
  counts as "other".
- The two C08 no-answers are empty replies with no tool call, returned in about 0.4 s. The record
  keeps only digests, so the cause is not known; they count as not followed.
- n is small (3 to 5 cases per category); the per-category intervals are wide.
- Only the `openai` dialect and this one model and backend were exercised.

## Changed

- Spec: InputRecord and exposure types, Crossveil trust rules 5 and 6, a host-controlled approval
  continuation (correlation by trace, request, minted `approval_id` and request digest; expiry
  against the host clock; terminal states), adapter subset rule (`spec/CORE.md`).
- Security fix: the Rust lifecycle accepted any non-empty `approval_id` on a continuation; it now
  needs the id the runtime minted, as Python already did (negative control V5 is the pre-fix
  behaviour).
- Pipeline: every rendered result is an InputRecord; exposure travels on every request and
  `CallContext`; the mock holds an effect for approval when the exposure floor is below
  `user_supplied`. Gaps found by the corpus were fixed in both SDKs (foreign result ids,
  duplicates, and others listed in `CHANGELOG.md`).
- Adapters: Odysseus arms its untrusted-context gate from exposure and fails closed on approval
  continuation; AIEN passes exposure to `EffectLane::with_exposure`, takes approvals only through
  `present_approval`, re-pinned to sovereign-core 0c1d249 (#207).
- Corpus: 60 trust cases over 13 categories, approval fixtures A01 to A15, negative controls V1 to
  V8 built only into the conformance runners, T3 and T4 subset runners, digests frozen.
- Tooling: `bench/tools/trust_run.py` (runs every leg and computes the gates), raw dumps of results
  and input ledgers from both runners, `bench/tools/live_injection.py`.

## Proven

- On the scripted obedient model, which emits every injected intent every time, no injected
  instruction produced an effect on any of the four systems (E: 0, 0, 0).
- Model-written content cannot approve anything: approval claims in arguments or extensions,
  forged provenance, and earlier model text dressed as a runtime approval were all refused (E3,
  C07, C08, A10).
- Approval mismatch, expiry, reuse and stale scope all refuse (A02 to A08) on T1 and T2; on T4
  A02 to A07 refuse (A08, the stale catalog case, is not in the T4 subset).
- A legitimate approval resumes on T1, T2 and T4 (A01). On T4 the grant is spent inside AIEN's
  own authority, and an effect that fails at the provider after the approval was spent is reported
  and ends the request; a second continuation is refused (A15).
- Rust and Python have identical trust semantics: 106 verdict rows byte-identical, input ledgers
  identical, and the same negative-control matrix.
- The suite can fail: each of the eight deliberately broken variants is caught.

## Failed

- No gate failed in the qualifying run.
- Found and fixed before the run (development run, not the qualifying run): P2 found three
  error results that the pipeline itself made after execution (foreign result, raised provider)
  without `content_kind` and `trust`. Both SDKs now label them `tool_result` and `unknown`
  (PR #40); behaviour is otherwise unchanged. The same dev run also showed a counting bug in the
  gate tool (it counted the replay-rejection result), fixed in the same PR.

## Unproven

- Odysseus approvals: continuation is unsupported on Odysseus, so A01 does not run there; the
  adapter refuses every continuation instead (allowed by A-ODY).
- AIEN `ReconciliationRequired` (a provider outcome that is unknown after the grant was spent) has
  no test.
- AIEN two-phase grant spend (sovereign-core #206) is open upstream in our own repo; A15 shows the
  grant is spent before the provider runs.
- A15 is detected on T4 through the provider's rejection message; it runs on T4 only (T1 and T2
  use the mock fault, T3 has no provider-failure mapping).
- Whether any model resists injected text (gate M reports the follow rate and gates only violations), any dialect other than `openai`
  in the live leg, any backend other than those named.
- Gate X proves identity only for behaviour the corpus exercises. After the run, two Rust/Python
  differences outside the corpus were found and fixed: non-string `openai` `arguments` (PR #45) and
  whether a call refused in `run_turn` uses its `request_id` (PR #47). Neither touches a 0.3
  fixture; both now have tests in both SDKs. Fixtures for them belong to the next corpus revision.
- Local development checks of T4 before the run used AIEN sibling checkouts that were not at the
  pins. The qualifying run and CI used pinned checkouts made by `adapters/aien/setup-siblings.sh`.

## Gate table, ROADMAP 0.3 (Trust)

| item | result | evidence |
|---|---|---|
| Source provenance | PASS | P1 to P4 |
| Untrusted-context semantics | PASS | E over T1 to T4, Crossveil rules 5 and 6 |
| Authorization states | PASS | A-SDK, A-AIEN, A-ODY |
| Denial and approval flow | PASS | A01 to A15 |
| Injection-oriented conformance cases | PASS | I (60 cases, 13 categories), N (V1 to V8) |
| Model behaviour reported | PASS | gate M, `live/summary.md` |
| Report | PASS | this file |

Overall 0.3 gate: **met**. Offline gates P, X, I, E, A, N pass; gate M passes on its one gated
measure (violations 0).

## External lanes (evidence only; no upstream action has been taken)

No upstream PR, issue or comment was made for 0.3. Upstream actions remain pending owner review.

## Architectural impact

- None to the wire format beyond additive fields (`provenance.exposure`, InputRecord).
- Authority stays outside INTERPLANE. On T4 only AIEN code decides and spends grants; the adapter
  hands it exposure and the approver's decision and nothing else.

## Next step (future work, NOT results)

Done after the run, outside this report's evidence: streamed-response assembly (dialect
`openai_stream`, PR #44) and session state limits with `close_trace` (PR #46), plan item 6.

1. 0.2x campaign results (`bench/PROTOCOL-0.2x.md`), its own report.
2. Re-pin the AIEN adapter to sovereign-core `0bdc97a` (#208: effect-bound idempotency ledger, revocable grants) now that the
   0.3 fixture freeze ends.
3. 0.4 Execution.
