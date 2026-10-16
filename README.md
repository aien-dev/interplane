# INTERPLANE

**INTERPLANE is an open interoperability protocol between AI models and agent runtimes.**

> Models express intent. Runtimes retain authority. INTERPLANE defines the boundary between them.

```
Model side:        Qwen3.5      Ajax (when published)      other local or hosted models
                        \            |             /
                         ----- INTERPLANE -----
                        /            |             \
Runtime side:           AIEN          other runtimes
```

INTERPLANE belongs to neither side. AIEN is the reference runtime (Odysseus was the first, through an
adapter removed on 2026-10-15; see "Removed adapters" below); Qwen3.5 is the first qualified model
dialect. Each runtime keeps its own tools, policy, approvals, memory and execution. INTERPLANE only
standardizes what crosses between them and makes it measurable.

## Try it offline (no model, no GPU, no network)

```
cd rust && cargo run -q -p interplane-crossveil --example offline
```

A scripted model fetches a web page that carries an injected instruction, then "obeys" it by
asking to send an email, write a file and append a note. The reference pipeline with its mock
runtime denies the email and the write, holds the note for a person's approval (untrusted content
is in view) and executes none of them. The example checks this and exits 1 on any difference. CI
runs it (job `rust`).

To connect your own program, see `docs/WRITING-AN-ADAPTER.md` and its runnable example
`rust/crates/interplane-crossveil/examples/todo_adapter.rs` (run it with
`cargo run -q -p interplane-crossveil --example todo_adapter`): a to-do list host with its own
policy, an approval a person grants and one they decline.

## What is in the repository (version 0.3.0)

The Rust workspace and the Python package are at version 0.3.0. The base is 0.1, the Reality Layer.
0.2 (Capability) is closed with its pre-registered gate failed and the failure published
(`docs/REPORT-0.2.md`). 0.3 (Trust) is closed with its gate passed (`docs/REPORT-0.3.md`). 0.4 and 0.5
are planned, not built (`ROADMAP.md`).

| Module | What it is | Status |
|---|---|---|
| **Core** | normative JSON Schemas (2020-12) for envelope, intent, capability, decision, result, event, catalog, selection, input record, probe report; versioning and error codes | see `STATUS.md` |
| **Lenshift** | model dialect parsers and result renderers: `openai`, `qwen35`; `ajax` reserved | see `STATUS.md` |
| **Crossveil** | the authority-boundary contract: two runtime callbacks, a lifecycle with fail-closed invariants, trust classes | see `STATUS.md` |
| **CrossAxis** | explicit, versioned capability mapping; a deterministic domain selector with a before/after receipt; bounded expansion | see `STATUS.md` |
| **Interplane Probe** | measured capability qualification of an OpenAI-compatible endpoint | see `STATUS.md` |
| **Conformance** | 106 conformance cases plus 56 dialect fixtures; Rust and Python runners must agree byte for byte | see `STATUS.md` |
| Vectorveil | the envelope and canonical JSON; no transport framework | envelope only |
| RelayLine | the event vocabulary for multi-round exchanges | contract only |

`STATUS.md` is the only place that says what is implemented, tested, or merely specified. Nothing
elsewhere in this repository claims more than `STATUS.md` does.

## Layout

```
spec/            normative Markdown + spec/schemas/*.schema.json
rust/            Cargo workspace: interplane-core, -lenshift, -crossaxis, -crossveil, -conformance, -probe
python/          interplane package (stdlib only at runtime)
dialects/        per-dialect fixtures
conformance/     canonical fixtures + runner contract
adapters/        aien/ reference adapter (depends on its host; Core never does); odysseus/ keeps catalog records only
examples/        receipt/ (before/after receipt); the offline and to-do examples are in rust/crates/interplane-crossveil/examples/
bench/           CrossAxis benchmark corpora, pre-registered protocols (0.2, 0.2x, 0.3), published runs (records only)
qualification/   real-model probe captures, evidence and reports
provenance/      companion manifest verifier, outside Core (ADR 0004)
docs/            reports (REPORT-*.md), WRITING-AN-ADAPTER.md, plans, analysis
docs/adr/        decisions
STATUS.md        what is implemented, tested, conformant or qualified
ROADMAP.md       the layers and where each stands
CURRENT_STATE.md what the host projects' live code actually contained when this started
```

## Removed adapters

Two Python adapters were removed on 2026-10-15 under the project rule that no Python ships here:

- `adapters/odysseus`: ran the Odysseus agent runtime as an INTERPLANE authority. It was the T3 runner
  of the closed 0.3 trust gate and the runtime of the 0.2 and 0.2x bench campaigns. Odysseus is a
  Python program, so the adapter could not move to Rust; its catalog records stay in
  `adapters/odysseus/` because the bench corpus and recorded runs cite them.
- `adapters/motus`: let a Lithos AI Motus agent (a Python framework) send its tool calls through
  INTERPLANE, using the AIEN authority through the `aien-authority-bridge` binary, which stays in
  `adapters/aien`.

The Rust AIEN adapter (`adapters/aien`) is the reference adapter. Both removed adapters remain in git
history (present through commit `43ec8f8`).

## Security in one paragraph

The model side is untrusted. Lenshift never executes. CrossAxis never grants. A valid envelope
means "structurally understandable", never "authorized". A signature proves origin, never
authority. The runtime makes every authorization decision through its own policy engine; INTERPLANE
carries the decision and refuses to upgrade it. Denied never becomes authorized by retry.

## License

Apache-2.0 for the protocol, schemas, fixtures and reference SDKs. `adapters/aien` depends on AGPL host
projects and is AGPL-3.0-or-later, with its own `LICENSE`, as are the Odysseus catalog records in
`adapters/odysseus`; see `docs/adr/0002-licensing.md`.
