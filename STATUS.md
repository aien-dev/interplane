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
| Lenshift aien_legacy | in progress | in progress | in progress | AIEN textual `<tool_call>` JSON dialect |
| Lenshift ajax | reserved | reserved | none | no official artifact (2026-10-04) |
| CrossAxis mapping + coercion | conformant | conformant | cases 04, 20, 21 | stale-digest refusal |
| CrossAxis Select (domain_match v1) | tested | tested, qualified | - | receipt: 71 -> 9 tools, 89% fewer first-turn prompt tokens |
| Crossveil pipeline + mock runtime | conformant | conformant | 23 cases | fails closed on adapter faults |
| Conformance runner | implemented | implemented | 23/23 both | `--dump` writes canonical results per case |
| Interplane Probe | tested (canned) | qualified | - | Python probe run against Qwen3.5-9B/Ollama 0.34.0; Rust probe not yet run live |
| AIEN adapter | in progress | - | - | ADR 0003: composes aien-mcp/aien-capability, mints nothing |
| Odysseus adapter | - | tested (55 tests vs odysseus@2992bf6) | - | executes read_file/ls/glob/grep via Odysseus handlers; AGPL |
| Before/after receipt (0.1 goal) | - | qualified | 2 runs, reproduced | examples/receipt |
| CI | specified | specified | - | workflow present; not yet run on GitHub (repo unpublished) |
