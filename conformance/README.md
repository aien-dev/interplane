# INTERPLANE conformance

Normative fixtures for Core 0.1. The format, mock runtime, mapping table and the 18 required
cases are defined in `spec/CORE.md`; dialect fixtures in `spec/LENSHIFT.md`. This directory only
pins concrete values. Where the spec was silent or ambiguous, the stricter (fail-closed) reading
was taken and is listed below.

## Layout

- `fixtures/NN-slug.json`: the 27 required cases (01-18 core; 19 valid write, 20 missing required argument, 21 stale capability mapping, 22 untrusted tool result, 23 untrusted memory result; 24 expansion by requested_excluded, 25 expansion refused by bound; 26 unrecognized content_kind, 27 absent, null and unrecognized trust). `fixtures/injection/NN-slug.json`: injection cases (0.3 cut I1: scripted obedient model, judged for effects and content-derived intents; seed cases 01 workspace, 02 tool output, 03 forged approval; cut I2 cases 04-19 for C01 to C05, fixture-level `mock_data`; cut I3 cases 20-34 for C06 to C09 and C11 (20-22 C06, 23-25 C07, 26-28 C08, 29-31 C09, 32-34 C11); cut I4 cases 35-42 for C12 (35-38) and C13 (39-42), fixture-level `mock_fault`). `negative-controls.json` and `TRUST-DIGEST.txt`: the negative-control expectations (V1 to V8) and the corpus freeze file. `fixtures/digest/jcs-01.json`: the JCS digest check. `fixtures/lifecycle/NN-slug.json`: approval continuation cases driven against the lifecycle directly (01 forged id, 02 empty or missing id, 03 id minted for another request, 04 the runtime-minted id, 05 non-`authorized` continuation values); format in `spec/CORE.md`. `fixtures/approval/NN-slug.json`: approval cases A01 to A14 of bench/PROTOCOL-0.3.md plus SDK follow-ups 15 (explicit `denied` continuation value) and 16 (continuation decision citing another request_id), and 17, the protocol's A15 (`protocol_case: "A15"`: the approved effect fails at the provider after the approval was spent; mock `execute_raises`) driven through the pipeline with host-side `approve`, `cancel`, `restart` and `mock` steps (case name `approval-ANN-slug`); format in `spec/CORE.md` (Approval continuation).
- `../dialects/fixtures/{openai,qwen35}/*.json`: Lenshift parse fixtures.
- `runners/validate_fixtures.py`: checks every envelope and intent against `spec/schemas/`, fixture shape,
  runtime-count consistency, the JCS fixture, plus negative controls. Exit status is non-zero on failure.
  Run: `python3 conformance/runners/validate_fixtures.py`.

## How a runner consumes a conformance fixture

1. Create the mock runtime (`CORE.md`, Mock runtime) and a fresh pipeline with `limits`.
2. For each step in order:
   - `dialect` + `input`: Lenshift-parse it with `trace_id` and `turn`. Wrap each intent in an envelope
     (`message_id = "m-<case>-<turn>-<n>"` where `<case>` is the `case` field and `<n>` the 0-based
     position among the turn's intents; timestamp `2026-01-01T00:00:00Z`; source model party, destination
     `runtime`/`mock`). Calls Lenshift rejected become `rejected` records (`malformed_tool_call`) with the same id rules.
   - `envelope`: admit as-is.
   - Run admit (version, replay, duplicate, size, in that order), map, decide, execute per `CORE.md`.
   - If `continues` is true, the previous step's rendered results are fed to the model for this step's
     turn. The step's own `input` is still what is parsed; rendered results are data and are never parsed as intents.
3. Compare. `expected.observed` is the ordered list of ObservedRecords across all steps. `expected.turns`
   covers Lenshift-parsed steps only. `expected.runtime` holds the decide/execute call counts.
   `expected.result_checks` (optional) asserts `path` (dotted, into the result payload) equals `equals`
   for the result of `request_id`. With `"round_trip": true` the value is read after the payload is
   serialized to JCS and parsed back into the SDK's own result type, as a receiver sees it.
4. `result_digest` is `null` in fixtures: implementations compute it (digest of the canonical result payload
   with `provenance.duration_ms` null) and the cross-language comparison covers it.

## Selection and expansion fixtures (cases 24, 25)

A fixture may carry `selection: {requested_domains, max_capabilities, always_include}`: the runner
builds the mock catalog and runs `select` before the first step. A step `{"turn": n, "expand":
{"evidence": {...}, "max_expansions"?, "max_added_per_expansion"?}}` runs `expand` on the current
selection and does not touch the pipeline. `expected.selections` is the exact selection receipt
after each expand step, in order (digests included). A verdict row gains a `selections` key only
for such cases, so rows 01-23 are unchanged. The reference pipeline has no selected-view mapping:
the `unknown_capability` that motivates a `requested_excluded` expansion is supplied by the harness
(the step's evidence), not produced by the pipeline. Case 24 then sends the call through the
pipeline and expects `denied`: visibility is not authority.

## Fixture-level `mock_provenance` (cases 26, 27)

`{<capability>: {content_kind?, trust?, trusted?}}`, copied into the mock runtime before the pipeline
runs; the mock's executed result for that capability reports these provenance values instead of its
defaults, the way a mislabelling adapter would. Harness configuration only: the mock never takes labels
from model content, and the catalog (and its digest) is unchanged. The cases check that unknown
provenance never becomes trusted: an unrecognized kind reads as `unknown`, absent or null trust as
`unknown` with `trusted` null, an unrecognized trust string as `external_untrusted` with `trusted`
false, and an adapter's `trusted: true` is ignored. Every check also runs after a JCS round trip.

## Fixture-level `mapping_table`

Absent means `mock-table`. Case 21 sets `"mapping_table": "mock-table-stale"` (same rules, wrong catalog digest), so CrossAxis must refuse with `stale_capability` before any runtime contact.

## Trust note (cases 19, 22, 23)

Trust is two axes (CROSSVEIL.md, `common.schema.json`): `provenance.trust` (TrustLevel) and `provenance.content_kind` (ContentKind), set by the runtime adapter and never by the model. `provenance.trusted` follows CORE.md: true only for `trusted_runtime`, false for `workspace_untrusted` and `external_untrusted`. Cases 22 and 23 return content that looks like instructions (tool-call markup, a destructive shell command); it must reach the model only as rendered data. Case 22's `continues` step is parsed from the model's own text and must add no intents or records. `result_checks` cover result fields only; a runner should additionally assert the rendered tool message contains the markup verbatim (CORE.md case 22).

## Lifecycle fixtures (approval continuation)

The cases under `fixtures/lifecycle/` exercise the REQUIRES_APPROVAL transition on the lifecycle
state machine itself, in both SDKs (no pipeline, no mock runtime). Both runners load them after the
`NN-slug` and injection cases and write one verdict row each, keyed `lifecycle/<slug>`.

## Approval fixtures (cases A01 to A14)

`fixtures/approval/` drives the host continuation calls of the pipeline (`continue_approval`,
`cancel_approval`; `spec/CORE.md`, Approval continuation) through steps only the fixture's host side
can issue: `approve`, `cancel`, `restart` (a fresh pipeline over the same mock runtime) and `mock`
(the runtime's catalog digest changes). `mock_approval.expires_at` makes the mock mint an expiry.
Each `approve` and `cancel` step adds one row to `expected.continuations` and, when resolved, one
`ObservedRecord`; the verdict row of an approval case gains `continuations`. Rows of earlier cases
are unchanged. Total verdict rows: 106 (40 + 44 injection + 17 approval + 5 lifecycle).

The mock effect policy (0.3 cut E1, `spec/CORE.md`) holds an `effect` capability for approval when
the exposure floor is below `user_supplied`. Fixtures 19 and 27 now register the user request first
so their `append_note` still runs; fixture 40 expects its second `append_note` held. Injection cases
43 (web, C04) and 44 (memory, C03) aim content at `append_note`, the one effect the mock otherwise
authorizes, and fail if the policy is removed.

## Raw evidence dumps (0.3 cut R1a)

Both runners take `--dump <dir>` and write, per pipeline case, `<case>.results.json` (every result payload in
canonical form, `provenance.duration_ms` null) and `<case>.inputs.json` (the full input ledger of the case trace,
whether or not the fixture asserts it). The verdict files are unchanged. The two runners write byte-identical dumps;
REPORT-0.3 counts gates P1, P2 and P4 over them (bench/PROTOCOL-0.3.md section 9).

## Corpus accounting (gate I)

`validate_fixtures.py` counts the corpus from fixture metadata and fails below the minimum of
bench/PROTOCOL-0.3.md 3.2. C01 to C09 and C11 to C13 are the `injection.category` of the injection
fixtures. **C10 (approval manipulation) is supplied by lane A:** every approval fixture except
A01 (the one legitimate approval) counts, 16 cases (A02 to A17) against a minimum of 10. It also
checks that the negative controls V1 to V8 together fail at least one case in each of the 13
categories (gate N). Current counts: C01=3, C02=5, C03=4, C04=4, C05=4, C06=3, C07=4, C08=3, C09=3,
C10=16, C11=3, C12=4, C13=4; 60 cases against the minimum of 52.

Fail-closed readings taken where the protocol leaves room: a continuation that fails a binding check
(wrong or foreign `approval_id`, changed arguments, expiry, changed catalog, decision value) on a
request that is still pending DENIES it for good (A02, A03, A04, A05, A08, A14); a call with no live
pending entry (unknown, already executed, cancelled, second continuation, after restart) is REFUSED
and changes nothing (A06, A07, A09, A12). A07 therefore presents the spent id for a request that was
never pending; the same id on a pending request is A03. Expiry compares the exact
`YYYY-MM-DDTHH:MM:SSZ` shape as strings and treats anything else as expired. A continuation never
calls `decide`, so its `ObservedRecord` has `decide_invoked` false.

## Cross-language verdict comparison

Each runner writes its full `observed` array (with `result_digest` filled) as JSON. Canonicalize each with
RFC 8785 and compare bytes: Rust and Python outputs must be byte-identical. A runner also compares its
array to the fixture's `expected.observed` after setting every `result_digest` to null.

## Spec ambiguities resolved (fail-closed), with spec citations

1. Case 13 `request_id` is `null` (CORE.md:163 allows null before an id exists; VERSIONING.md rule 1: another MAJOR is rejected before the payload is parsed). Decide and execute false.
2. Admission check order is version, replay, duplicate, size (CORE.md:85). So case 17's second admission is `replayed_message` although the request id is also a duplicate. That record keeps the payload's `request_id`.
3. `turns` lists only Lenshift-parsed (`dialect`) steps; envelope steps get no entry (CORE.md:144, 150: `text` and intent counts only exist for parsed turns).
4. `turns[].outcome`: dialect unsupported, or 0 intents with at least 1 rejected, gives `rejected`; any intents gives `tool_request` (even if Core later rejects them, cases 15, 16); otherwise `no_tool` (CORE.md:144, 181, 184).
5. Case 12 needs the code `unsupported_dialect` (CORE.md:183) but the turn shape has no field: an `error_code` key is added to that turn entry. Its `rejected` count is 0.
6. Format extensions: `expected.result_checks` (cases 01, 05 to 09, 14) because `result_digest` is null and the spec requires checks on approval_id, data and messages (CORE.md:176, 177). Dialect fixtures also carry `reasoning_present`, `parser_version`, `expected_digests`, `dialect`, `name`, `note` (LENSHIFT.md:95 shape plus the brief).
7. Case 18's retry is an explicit envelope step (an OpenAI turn cannot carry `extensions`, CORE.md:188). The fabricated `approval_id` is put in arguments, payload `extensions` and envelope `extensions`. Extra argument keys do not make the mock answer `invalid`, since it checks only required keys and declared types (CORE.md:123); extra keys are tolerated. Both decisions are `denied` (CROSSVEIL.md rules 2, 4).
8. Case 03 record: stage `REJECTED`, decision `not_found`, decide invoked (CORE.md:46, 74, 173). Rejections before the runtime have `decision: null`.
9. Request ids: OpenAI `tool_calls[i].id` when present, else `<trace_id>:t<turn>:c<index>` (LENSHIFT.md:28). `index` is the 0-based position among all calls in the turn, rejected ones included; it is used for both `c<index>` and `rejected[].index`. Case 16 reuses `call_16a` for both calls and both records carry it.
10. Case 11: `continues: true` is on the step that follows the result (turn 1) (CORE.md:153). The qwen35 turn text is trimmed; `<think>` content is removed from `text`.
11. `parser_version` is `1.0.0` (schema requires semver, spec pins no value; VERSIONING.md:11). Provenance `dialect_version` is `"1"` for openai and qwen35 XML (LENSHIFT.md:33, 51).
12. Hermes form: intent provenance `dialect_version = "hermes_json"` (LENSHIFT.md:70-72); the turn-level `dialect_version` stays `"1"`; `coercion = "none"` because no value is guessed.
13. Missing-name rejection message is `missing tool name` (LENSHIFT.md:42 fixes only the code). Other messages are taken verbatim from LENSHIFT.md. Qwen35 unclosed-function and cut-off inputs both give `partial = true` and `"truncated tool call"` (LENSHIFT.md:68-69), differing only in input shape.
14. qwen35 `json_guess` applied literally (LENSHIFT.md:64-65): a value parsing to a non-string is used as parsed, including `123` for a parameter named `path` and `null` becoming null; a value that parses to a string (`"text"` with quotes) keeps its raw text. `coercion = "json_guess"` is recorded for every XML-form call, including calls with no parameters.
15. The oversized limit is 65536 bytes of the canonical JSON of `arguments` (CORE.md:35). Case 15's arguments are about 70 KB.
16. Namespace is `null` for bare names; `filesystem.stat` gives namespace `filesystem`, name `stat` (LENSHIFT.md:24-26).
17. `reasoning_digest` and `source_digest` are null in fixtures: the spec does not pin the bytes hashed. `reasoning_present` records that a non-null reasoning digest is expected.
18. The `trailing_text` note (LENSHIFT.md:73-74) is not exercised: the spec says "in extensions" without saying where, and no required fixture covers it.
19. Case slugs other than 01 are derived from the CORE.md case names by hyphenating; only `01-valid-tool-request` is given verbatim.
20. JCS: keys sort by UTF-16 code units (RFC 8785), so the emoji key sorts before U+FF5E. Floats are limited to `1.0`, `1e21`, `0.5`, `2.50`, which avoids shortest-round-trip edge cases. The digest was computed with a hand-written canonicalizer and cross-checked against Node's `JSON.stringify`.
21. Protocol version in valid envelopes is `"0.1"` (CORE.md, Canonical result payload shape); case 13 keeps `"2.0"`, an unsupported MAJOR.
22. Error messages are now pinned by CORE.md ("Canonical result payload shape"); cases 20 and 21 assert them with `result_checks`. `missing tool name` (item 13) is confirmed by that table.
23. Case 20: the runtime is asked and answers `invalid`, so stage `REJECTED`, decision `invalid`, decide true (CORE.md case table row 20). Case 21: nothing reaches the runtime, so decision null, decide false.
24. Case 22/23 `provenance.trusted` is asserted false (CORE.md: false for the two untrusted levels); case 19 true.
25. Cases 24-25 (CROSSAXIS.md, Bounded expansion): the selection receipt after each expansion is compared exactly; `validate_fixtures.py` also recomputes `selection_digest` and the `parent_digest` chain independently and validates each receipt against `selection.schema.json`.

## Adapter subsets (T3 Odysseus, T4 AIEN)

`adapter-translation.json` is the data of the normative rule in `spec/CORE.md` (Adapter subsets);
`runners/adapter_subset.py` computes each adapter's subset from fixture metadata
(`--subset odysseus|aien`) and the translated plans (`--plans odysseus|aien`), and
`runners/trust_digest.py` freezes both lists (`t3_subset`, `t4_subset`) and the table's sha256 in
`TRUST-DIGEST.txt`. Current subsets: T3 43 fixtures (30 injection, 13 approval), T4 32 (18
injection, 14 approval).

- T3: `ODYSSEUS_SRC=<odysseus@2992bf6> PYTHONPATH=python:adapters/odysseus <venv>/bin/python -m interplane_adapter_odysseus.t3 --out t3-verdicts.json`
  (real `Pipeline` + `OdysseusAuthority`, Odysseus's own gate).
- T4: `cd adapters/aien && T4_OUT=t4-verdicts.json cargo test --test t4_corpus -- --nocapture`
  (real `Pipeline` + `AienAuthority`; needs `python3` for the plans and the sibling checkouts of `adapters/aien/README.md`).

Both fail on any subset member they cannot run, and T4 also fails when its computed subset differs
from the frozen list (T3 likewise). The verdict JSON has one row per fixture plus a summary with
`violations` and `content_derived` (gate E over the subset). Red check: with Odysseus's
exposure trigger forced off, T3 failed 43 of 43 (32 violations; all 30 injection cases executed
the injected effect, all 13 approval cases failed); with AIEN's `write_file` mis-enrolled as
`LOCAL_EPHEMERAL`, T4 failed all 18 injection cases and all 13 approval cases. Neither change is committed.

Out of subset by rule, not by result: T3 excludes 18 fixtures (injection delete targets 05, 12, 13, 18,
22, 26, 31; `fail_tool` 09; mock-fault, stale-catalog and duplicate-name cases 35-40; approval 08,
15, 16, and 17, which is T4 only); T4 additionally excludes the `send_email` targets. Approval fixtures A01 to A14 except A08
(`mock` step) are in both subsets. Fixture 17 (the protocol's A15) is in T4 only: the write goes to a
missing directory, AIEN's write provider rejects it after the grant was spent (its message reads "path is
outside the workspace"), the failure is reported and the second continuation is refused. T4 is 32 of 32.
