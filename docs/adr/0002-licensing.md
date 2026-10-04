# ADR 0002: Licensing

Status: Accepted (2026-10-04)

Facts checked on 2026-10-04: Odysseus is AGPL-3.0; the AIEN repositories are AGPL-3.0-or-later
(relicensed 2026-10-04); the Qwen3.5 chat template is Apache-2.0.

Decision:

- Protocol, schemas, specifications, fixtures, the Rust crates, the Python package, examples and
  the probe: **Apache-2.0**. Reason: the patent grant and permissive terms make adoption by any
  runtime, including proprietary and AGPL ones, frictionless. AGPL projects may incorporate
  Apache-2.0 code (one-way compatibility), so both hosts can vendor or depend on Core.
- `adapters/aien` and `adapters/odysseus`: **AGPL-3.0-or-later**, each with its own LICENSE file,
  because they link to or import their AGPL hosts. They are reference code, not Core.
- No code is copied from either host into Core. Adapters may depend on their hosts; Core never does.
- The Qwen3.5 chat template is stored under `qualification/evidence/` for reference with its
  Apache-2.0 notice; no model weights are stored.
