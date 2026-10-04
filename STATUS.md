# STATUS (updated by hand with every merge; the readiness report cites this)

Statuses: `specified` (normative text/schema exists) · `implemented` (code exists) · `tested`
(unit tests pass) · `conformant` (passes the canonical fixtures in both languages) · `qualified`
(measured against a real model/backend).

| Component | Rust | Python | Fixtures | Notes |
|---|---|---|---|---|
| Core schemas | specified | specified | - | 10 schemas under spec/schemas |
| Core types + validation | planned | planned | planned | |
| Lenshift openai | planned | planned | planned | |
| Lenshift qwen35 | planned | planned | planned | |
| Lenshift ajax | reserved | reserved | none | no official artifact (2026-10-04) |
| CrossAxis mapping | planned | planned | planned | |
| CrossAxis Select | planned | planned | planned | minimal domain selector |
| Crossveil pipeline + mock runtime | planned | planned | planned | |
| Conformance runner | planned | planned | 18 cases planned | |
| Interplane Probe | planned | - | - | Rust CLI |
| AIEN adapter | planned | - | - | |
| Odysseus adapter | - | planned | - | |
| CI | planned | | | |
