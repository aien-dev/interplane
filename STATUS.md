# STATUS (updated by hand with every merge; the readiness report cites this)

Statuses: `specified` (normative text/schema exists) · `implemented` (code exists) · `tested`
(unit tests pass) · `conformant` (passes the canonical fixtures in both languages, verdicts
byte-identical) · `qualified` (measured against a real model/backend).

| Component | Rust | Python | Fixtures | Notes |
|---|---|---|---|---|
| Core schemas | specified | specified | validated | 10 schemas under spec/schemas, JSON Schema 2020-12 |
| Core types + validation + JCS | conformant | conformant | 25 cases + digest/jcs-01 | verdict files byte-identical |
| Lenshift openai | conformant | conformant | 8 dialect fixtures | |
| Lenshift qwen35 | conformant, qualified | conformant, qualified | 10 dialect fixtures + real captures | real Qwen3.5-9B captures under qualification/captures |
| Lenshift aien_legacy | tested | tested | 13 dialect fixtures, JCS-identical | aien-cli textual `<tool_call>` JSON (client.rs:381-419); unterminated calls rejected |
| Lenshift ajax | reserved | reserved | none | no official artifact (2026-10-04) |
| CrossAxis mapping + coercion | conformant | conformant | cases 04, 20, 21 | stale-digest refusal |
| CrossAxis Select (domain_match v1) | tested | tested, qualified | - | receipt: 71 -> 9 tools, 89% fewer first-turn prompt tokens (0.1 measure: rendered bytes + model-reported tokens) |
| CrossAxis measure units (bytes, tokens_model_reported, tokens_endpoint_tokenizer, tokens_estimated) | tested | tested | schema-validated (selection.schema.json) | not yet qualified: Core ships BytesOnly + EstimatedTokens (estimator v1); live counters belong to the bench runner |
| CrossAxis expansion v1 (expand + discover, bounded) | conformant | conformant | cases 24, 25 | selections and digests byte-identical across languages; not yet qualified against a live model; never grants authority |
| Crossveil pipeline + mock runtime | conformant | conformant | 25 cases | fails closed on adapter faults |
| Conformance runner | implemented | implemented | 25/25 both (rerun 2026-10-04, verdicts byte-identical) | `--dump` writes canonical results per case |
| Interplane Probe | qualified | qualified | - | both run live against Qwen3.5-9B/Ollama 0.34.0 with identical verdicts (reports under qualification/reports) |
| AIEN adapter | tested (on the real EffectAuthority path, pinned sovereign-core 6554aac; was 10 pipeline tests in 0.1) | - | - | ADR 0003; real aien-mcp SpeculativeLane + aegis gate; aien-cli SafetyEngine not linkable (private); AGPL |
| Odysseus adapter | - | tested (55 tests vs odysseus@2992bf6) | - | executes read_file/ls/glob/grep via Odysseus handlers; AGPL |
| Before/after receipt (0.1 goal) | - | qualified | 2 runs, reproduced | examples/receipt |
| CrossAxis gate, 0.2 pre-registered (37 pairs, qwen3.5:9b on Ollama) | - | qualified: FAIL | bench/runs/qual-20261004T2207Z | T 0.877 PASS, S PASS (B 0.730 vs A 0.676), O1 0.865 FAIL, O2 0/5 FAIL; model did not use the discovery tool. See docs/REPORT-0.2.md |
| Qwen3.5-9B on llama.cpp b11398 | - | qualified | probes 14/14 (Rust and Python, aligned) | qualification/captures/qwen35-llamacpp |
| Qwen3.5-9B on SGLang 0.5.20 (bf16, qwen3_coder parser) | - | qualified | probes 14/14 (Rust and Python, aligned) | earlier 12/14 kept as pre-alignment |
| AIEN authority path (sovereign-core #203, #204) | tested | - | adapter approval tests (6) | single-use approvals; grants in memory, spent at mint, host clock |
| CI | running | running | - | github.com/aien-dev/interplane: schemas, rust, python, cross-language identity, both adapters |
