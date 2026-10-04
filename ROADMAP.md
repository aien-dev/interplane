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

## 0.2 sequencing (Drake, 2026-10-04): evidence before architecture

0.2 adds no new architecture. In order: (1) documentation precision fixes; (2) human-submitted
Odysseus #6474; (3) the 30–50 task CrossAxis benchmark (easy filesystem, ambiguous, multi-domain,
rare tool, wrong-first-tool, tool failures, denied effects, injection in workspace/tool content,
selection that must expand after round one, unknown-tool recovery, sequential tools), reported as
distributions: median tool reduction, median prompt-token reduction, success-rate difference,
missing-required-tool rate, unnecessary-tool-call rate, median rounds, wall-clock; gate target
(not tuned to): >= 70% less exposed tool-schema cost with success statistically indistinguishable
from the full catalog and required-tool omission below a predefined threshold; (4) Qwen3.5
qualified on llama.cpp and SGLang; (5) close the AIEN production-authority seam (adapter ->
aien-mcp -> real authorization -> AuthorizedEffect -> EffectLane); (6) publish the 0.2 Capability
Report; (7) only then approach Odysseus with INTERPLANE itself. The repository stays under
`aien-dev` through 0.2; neutrality is demonstrated technically first.

## 0.2 closed (2026-10-04): gate FAIL recorded

0.2 is closed with the pre-registered CrossAxis gate FAILED (T and S passed; O1 0.865 and O2 0/5
failed because the model never used the discovery tool). Thresholds were not changed. See
`docs/REPORT-0.2.md`. Qwen3.5 is qualified on llama.cpp and SGLang; the AIEN authority path is real
with listed limits.

Next: **0.3 Trust**. In parallel, **0.2.x expansion follow-up** as a new pre-registered protocol
(runtime-triggered expansion on unknown tool, failed call or approval; discovery prompting;
reasoning off). That is planned work, not a result.
