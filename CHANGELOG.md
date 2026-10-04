# Changelog

## Unreleased (0.1.0)

- Phase 0 audit (`CURRENT_STATE.md`).
- Core schemas, specs for Core, Lenshift, Crossveil, CrossAxis, Probe, Vectorveil, RelayLine, versioning.
- Conformance corpus: 23 cases + JCS digest fixture; Rust and Python runners agree byte-for-byte.
- Rust crates and Python package (stdlib-only) for Core, Lenshift (openai, qwen35), CrossAxis, Crossveil.
- Interplane Probe (Rust crate; Python reference run live against Qwen3.5-9B on Ollama 0.34.0).
- Real Qwen3.5-9B captures (native, raw, streaming, result replay).
- External Odysseus adapter (AGPL-3.0-or-later), 55 tests against the real Odysseus code.
- Before/after receipt demo: 71 -> 9 tools, 89.4% fewer first-turn prompt tokens, same task success.
- Lenshift `aien_legacy` dialect (aien-cli textual form), 13 fixtures, identical in both languages.
- AIEN reference adapter (AGPL-3.0-or-later), 10 pipeline tests through real aien-mcp and aegis code.
- Both probes run live with identical verdicts; CI covers both adapters with pinned upstream checkouts.
