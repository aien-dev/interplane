# STATUS (updated by hand with every merge; the readiness report cites this)

Statuses: `specified` (normative text/schema exists) · `implemented` (code exists) · `tested`
(unit tests pass) · `conformant` (passes the canonical fixtures in both languages, verdicts
byte-identical) · `qualified` (measured against a real model/backend).

| Component | Rust | Python | Fixtures | Notes |
|---|---|---|---|---|
| Core schemas | specified | specified | validated | 10 schemas under spec/schemas, JSON Schema 2020-12 |
| Core types + validation + JCS | conformant | conformant | 23 cases + digest/jcs-01 | verdict files byte-identical |
| Lenshift openai | conformant | conformant | 8 dialect fixtures | |
| Lenshift qwen35 | conformant, qualified | conformant, qualified | 10 dialect fixtures + real captures | real Qwen3.5-9B captures under qualification/captures |
| Lenshift aien_legacy | tested | tested | 13 dialect fixtures, JCS-identical | aien-cli textual `<tool_call>` JSON (client.rs:381-419); unterminated calls rejected |
| Lenshift ajax | reserved | reserved | none | no official artifact (2026-10-04) |
| CrossAxis mapping + coercion | conformant | conformant | cases 04, 20, 21 | stale-digest refusal |
| CrossAxis Select (domain_match v1) | tested | tested, qualified | - | receipt: 71 -> 9 tools, 89% fewer first-turn prompt tokens |
| Crossveil pipeline + mock runtime | conformant | conformant | 23 cases | fails closed on adapter faults |
| Conformance runner | implemented | implemented | 23/23 both | `--dump` writes canonical results per case |
| Interplane Probe | qualified | qualified | - | both run live against Qwen3.5-9B/Ollama 0.34.0 with identical verdicts (reports under qualification/reports) |
| AIEN adapter | tested (10 pipeline tests) | - | - | ADR 0003; real aien-mcp SpeculativeLane + aegis gate; aien-cli SafetyEngine not linkable (private); AGPL |
| Odysseus adapter | - | tested (55 tests vs odysseus@2992bf6) | - | executes read_file/ls/glob/grep via Odysseus handlers; AGPL |
| Before/after receipt (0.1 goal) | - | qualified | 2 runs, reproduced | examples/receipt |
| CI | running | running | - | github.com/aien-dev/interplane: schemas, rust, python, cross-language identity, both adapters |
