# ADR 0004: Provenance is a companion manifest outside Core

Status: Proposed (2026-10-06)

Context: a runtime effect should be traceable back to the model that proposed it and to that
model's training data and licenses. The evidence already exists in separate records: WALDO
corpus/run/model/release BOMs, the AIEN candidate manifest and daemon load line, INTERPLANE
envelopes, and the aien-cli effect receipt. Core must stay independent of AIEN and of any trainer
(ADR 0001), and VERSIONING.md says unknown `extensions` are never acted on.

Decision: a separate top-level `provenance/` crate (Apache-2.0, own Cargo workspace like
`adapters/aien`, depends only on `interplane-core`) defines `COMPANION.json` schema 0 and an
offline verifier. It references immutable records by retained bytes and digests and reuses each
record's own identity. It adds no envelope field, no `extensions` key, no message kind and no
protocol version change; Core and the conformance corpus are unchanged.

Consequences: no wire contract changes, so no MINOR/MAJOR bump. Provenance stays evidence: no
verdict feeds a decision. A later move into Core (for example a provenance reference in
`extensions`) needs its own ADR, fixtures in both languages and a version decision.
