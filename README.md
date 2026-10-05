# INTERPLANE

**INTERPLANE is an open interoperability protocol between AI models and agent runtimes.**

> Models express intent. Runtimes retain authority. INTERPLANE defines the boundary between them.

```
Model side:        Qwen3.5      Ajax (when published)      other local or hosted models
                        \            |             /
                         ----- INTERPLANE -----
                        /            |             \
Runtime side:      Odysseus        AIEN         other runtimes
```

INTERPLANE belongs to neither side. Odysseus and AIEN are the first two reference runtimes; Qwen3.5
is the first qualified model dialect. Both runtimes keep their own tools, policy, approvals, memory
and execution. INTERPLANE only standardizes what crosses between them and makes it measurable.

## Try it offline (no model, no GPU, no network)

```
python3 examples/offline/run_offline.py
```

A scripted model fetches a web page that carries an injected instruction, then "obeys" it by
asking to send an email, write a file and append a note. The reference pipeline with its mock
runtime denies the email and the write, holds the note for a person's approval (untrusted content
is in view) and executes none of them. The script checks this and exits 1 on any difference. It
uses only the Python standard library. CI runs it against the built wheel installed in a clean
environment (job `package`).

To connect your own program, see `docs/WRITING-AN-ADAPTER.md` and its runnable example
`examples/adapter/todo_adapter.py`: a to-do list host with its own policy, an approval a person
grants and one they decline.

## What is in 0.1 (the Reality Layer)

| Module | What it is | Status |
|---|---|---|
| **Core** | normative JSON Schemas (2020-12) for envelope, intent, capability, decision, result, event, catalog, selection, probe report; versioning and error codes | see `STATUS.md` |
| **Lenshift** | model dialect parsers and result renderers: `openai`, `qwen35`; `ajax` reserved | see `STATUS.md` |
| **Crossveil** | the authority-boundary contract: two runtime callbacks, a lifecycle with fail-closed invariants, trust classes | see `STATUS.md` |
| **CrossAxis** | explicit, versioned capability mapping; a deterministic domain selector with a before/after receipt | minimal in 0.1 |
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
adapters/        aien/, odysseus/ reference adapters (depend on their hosts; Core never does)
examples/        minimal mock runtime and mock model
docs/adr/        decisions
CURRENT_STATE.md what the host projects' live code actually contained when this started
```

## Security in one paragraph

The model side is untrusted. Lenshift never executes. CrossAxis never grants. A valid envelope
means "structurally understandable", never "authorized". A signature proves origin, never
authority. The runtime makes every authorization decision through its own policy engine; INTERPLANE
carries the decision and refuses to upgrade it. Denied never becomes authorized by retry.

## License

Apache-2.0 for the protocol, schemas, fixtures and reference SDKs. The reference adapters under
`adapters/` depend on their host projects and are licensed to match them; see `docs/adr/0002-licensing.md`.
