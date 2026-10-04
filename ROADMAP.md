# INTERPLANE roadmap

| Version | Layer | Builds | Goal |
|---|---|---|---|
| 0.1 | Reality | Core schemas, Lenshift (openai, qwen35), Interplane Probe, basic Crossveil, conformance suite, AIEN + Odysseus reference adapters | Prove what a model/backend/runtime combination really supports |
| 0.2 | Capability | CrossAxis discovery, capability ranking, minimal tool exposure (Select), prompt-token accounting, task-success regression suite | Give small local models the minimum tools required without making them less capable |
| 0.3 | Trust | source provenance, untrusted-context semantics, authorization states, denial/approval flow, injection-oriented conformance cases | Preserve the line between content, model intent and runtime authority |
| 0.4 | Execution | Vectorveil transport, RelayLine streaming, cancellation, continuation, retries, durable execution receipts | Multi-round execution without losing the boundary |
| 0.5 | Qualification | Interplane Bench, model/backend profiles, speculative-decoding qualification, hardware presets, Cookbook-consumable evidence | Measured, reproducible model and backend claims |

0.1 ships the schemas for `selection` and the trust classes already, because the wire format must
not change when 0.2 and 0.3 land; the executable parts of those layers in 0.1 are deliberately
minimal (a deterministic domain selector, trust tagging on results) and are labelled as such.

Out of scope on purpose: UI, CSS, editors, mobile, accessibility, email transport performance,
backup/restore, scheduler UX. Help for those goes to the host projects directly.

Two tracks run in parallel: **A**, small direct contributions to the host projects (bugs reproduced,
fixed narrowly, with regression tests); **B**, INTERPLANE itself, which turns every such bug into a
fixture or probe. Ajax is added only from official artifacts (see `spec/LENSHIFT.md`).
