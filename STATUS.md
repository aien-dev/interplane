# STATUS (updated by hand with every merge; the readiness report cites this)

Statuses: `specified` (normative text/schema exists) · `implemented` (code exists) · `tested`
(unit tests pass) · `conformant` (passes the canonical fixtures in both languages, verdicts
byte-identical) · `qualified` (measured against a real model/backend).

| Component | Rust | Python | Fixtures | Notes |
|---|---|---|---|---|
| Core schemas | specified | specified | validated | 10 schemas under spec/schemas, JSON Schema 2020-12 |
| Core types + validation + JCS | conformant | conformant | 27 cases + digest/jcs-01 | verdict files byte-identical |
| Lenshift openai | conformant | conformant | 8 dialect fixtures | |
| Lenshift qwen35 | conformant, qualified | conformant, qualified | 10 dialect fixtures + real captures | real Qwen3.5-9B captures under qualification/captures |
| Lenshift aien_legacy | tested | tested | 13 dialect fixtures, JCS-identical | aien-cli textual `<tool_call>` JSON (client.rs:381-419); unterminated calls rejected |
| Lenshift ajax | reserved | reserved | none | no official artifact (2026-10-04) |
| CrossAxis mapping + coercion | conformant | conformant | cases 04, 20, 21 | stale-digest refusal |
| CrossAxis Select (domain_match v1) | tested | tested, qualified | - | receipt: 71 -> 9 tools, 89% fewer first-turn prompt tokens (0.1 measure: rendered bytes + model-reported tokens) |
| CrossAxis measure units (bytes, tokens_model_reported, tokens_endpoint_tokenizer, tokens_estimated) | tested | tested | schema-validated (selection.schema.json) | not yet qualified: Core ships BytesOnly + EstimatedTokens (estimator v1); live counters belong to the bench runner |
| CrossAxis expansion v1 (expand + discover, bounded) | conformant | conformant | cases 24, 25 | selections and digests byte-identical across languages; not yet qualified against a live model; never grants authority |
| Crossveil pipeline + mock runtime | conformant | conformant | 27 cases | fails closed on adapter faults |
| Crossveil trust vocabulary (0.3 cut P1) | conformant | conformant | cases 26, 27 | unrecognized content_kind -> `unknown` in both SDKs (Python used `tool_result` before); absent/null trust -> `unknown`, unrecognized -> `external_untrusted`; `trusted` never true for them, also after a JCS round trip; Odysseus `system` integrity mapped explicitly |
| InputRecord and Exposure types (0.3 cut P2) | conformant | conformant | digest/input-record-01, digest/exposure-01 | schema `input.schema.json`, 9 source-class examples validated in CI; types; the pipeline now records them (cut P3, next row) |
| Crossveil lifecycle: approval continuation | conformant | conformant | lifecycle/01-05, approval/A01-A14 | only the approval_id the runtime minted for that request continues it; forged, empty, missing and other-request ids refused (Rust accepted any non-empty id before 0.3 cut A1); correlation is trace + request + minted id + capability_request digest, with expiry, cancel, replay and restart refusals; host-only `continue_approval` / `cancel_approval` on both pipelines (cut A2) |
| Reproducibility pins | - | - | - | see docs/RUN-MANIFEST-COMMITS.md (toolchain, pip constraints, action SHAs, AIEN sibling pins) |
| Pipeline input ledger and exposure (0.3 cut P3) | conformant | conformant | cases 28-30 | per-trace ledger of rendered results plus host `register_input`; every `tool_request` and `CallContext` gets a computed `exposure`; forged exposure overwritten; fails closed |
| Source-class coverage (0.3 cut P4) | conformant | conformant | cases 31-40 | one fixture per source class asserting every InputRecord field; mock gains `read_document`, `load_skill`, `call_provider`; skills and tool descriptions are `external_untrusted`; host registers model_generated input |
| Conformance runner | implemented | implemented | 45/45 both (40 cases + 5 lifecycle, verdicts byte-identical) | `--dump` writes canonical results per case |
| Interplane Probe | qualified | qualified | - | both run live against Qwen3.5-9B/Ollama 0.34.0 with identical verdicts (reports under qualification/reports) |
| AIEN adapter | tested (on the real EffectAuthority path, pinned sovereign-core 6554aac; was 10 pipeline tests in 0.1) | - | - | ADR 0003; real aien-mcp SpeculativeLane + aegis gate; aien-cli SafetyEngine not linkable (private); AGPL |
| Odysseus adapter | - | tested (70 tests vs odysseus@2992bf6) | - | executes read_file/ls/glob/grep via Odysseus handlers; approval continuation unsupported, every continuation refused (cut A4); AGPL |
| Before/after receipt (0.1 goal) | - | qualified | 2 runs, reproduced | examples/receipt |
| CrossAxis gate, 0.2 pre-registered (37 pairs, qwen3.5:9b on Ollama) | - | qualified: FAIL | bench/runs/qual-20261004T2207Z | T 0.877 PASS, S PASS (B 0.730 vs A 0.676), O1 0.865 FAIL, O2 0/5 FAIL; model did not use the discovery tool. See docs/REPORT-0.2.md. 0.2x held-out campaign (bench/PROTOCOL-0.2x.md) not run: runner prepared; arm 3 negative controls N1 and N2 (denied and approval-pending calls cause no expansion and no execution) tested offline in bench/tools/test_runner_offline.py |
| Qwen3.5-9B on llama.cpp b11398 | - | qualified | probes 14/14 (Rust and Python, aligned) | qualification/captures/qwen35-llamacpp |
| Qwen3.5-9B on SGLang 0.5.20 (bf16, qwen3_coder parser) | - | qualified | probes 14/14 (Rust and Python, aligned) | earlier 12/14 kept as pre-alignment |
| AIEN authority path (sovereign-core #203, #204) | tested | - | adapter approval tests (15; 6 original plus A3) | single-use approvals; grants are presented only through the host continuation API (cut A3), spent at mint, host clock; spend-point limit in adapters/aien/README.md |
| Injection harness + negative controls (0.3 cut I1) | tested | tested | 3 seed injection cases | scripted obedient model, effect judge, V1-V6 built only into the runners (Rust feature, Python flag), matrix byte-identical in both languages, `conformance/TRUST-DIGEST.txt`; category corpora are I2-I4, gate N needs them |
| Content-borne injection corpus (0.3 cut I2) | tested | tested | injection 04-19 (C01 to C05, forms F1 to F3) | 19 injection cases, all blocked by the runtime layer (no runtime gap); fixture `mock_data` harness override; V1 and V4 extended |
| CI | running | running | - | github.com/aien-dev/interplane: schemas, rust, python, cross-language identity, both adapters |
