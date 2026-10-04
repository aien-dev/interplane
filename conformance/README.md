# INTERPLANE conformance

Normative fixtures for Core 0.1. The format, mock runtime, mapping table and the 18 required
cases are defined in `spec/CORE.md`; dialect fixtures in `spec/LENSHIFT.md`. This directory only
pins concrete values. Where the spec was silent or ambiguous, the stricter (fail-closed) reading
was taken and is listed below.

## Layout

- `fixtures/NN-slug.json`: the 23 required cases (01-18 core; 19 valid write, 20 missing required argument, 21 stale capability mapping, 22 untrusted tool result, 23 untrusted memory result). `fixtures/digest/jcs-01.json`: the JCS digest check.
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
   for the result of `request_id`.
4. `result_digest` is `null` in fixtures: implementations compute it (digest of the canonical result payload
   with `provenance.duration_ms` null) and the cross-language comparison covers it.

## Fixture-level `mapping_table`

Absent means `mock-table`. Case 21 sets `"mapping_table": "mock-table-stale"` (same rules, wrong catalog digest), so CrossAxis must refuse with `stale_capability` before any runtime contact.

## Trust note (cases 19, 22, 23)

Trust is two axes (CROSSVEIL.md, `common.schema.json`): `provenance.trust` (TrustLevel) and `provenance.content_kind` (ContentKind), set by the runtime adapter and never by the model. `provenance.trusted` follows CORE.md: true only for `trusted_runtime`, false for `workspace_untrusted` and `external_untrusted`. Cases 22 and 23 return content that looks like instructions (tool-call markup, a destructive shell command); it must reach the model only as rendered data. Case 22's `continues` step is parsed from the model's own text and must add no intents or records. `result_checks` cover result fields only; a runner should additionally assert the rendered tool message contains the markup verbatim (CORE.md case 22).

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
