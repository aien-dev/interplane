# Odysseus catalog records (adapter removed 2026-10-15)

This folder used to hold `interplane-adapter-odysseus`, a Python adapter that ran
[Odysseus](https://github.com/odysseus-dev/odysseus) (pinned at `a8c147b`, earlier `2992bf6`) as an
INTERPLANE runtime authority. It served as the T3 runner of the 0.3 trust gate
(`bench/PROTOCOL-0.3.md`, recorded in `bench/runs/trust-0.3-20261005T0229Z`) and as the runtime of
the 0.2 and 0.2x bench campaigns and of `examples/receipt`.

It was removed on 2026-10-15 under the project rule that no Python ships in this repository. Odysseus
is a Python program, so an adapter that imports it cannot be rewritten in Rust; INTERPLANE's
reference adapter is now the Rust AIEN adapter (`adapters/aien`), and the Rust to-do host
(`rust/crates/interplane-crossveil/examples/todo_adapter.rs`) shows how to write a new one. The last
commit with the adapter is `43ec8f8`; its code and tests remain in git history.

What stays here is data that the recorded runs and the bench corpus cite, unchanged:

| File | What it is |
|---|---|
| `catalog.odysseus-2992bf6.json` | The frozen 0.2 corpus catalog (71 tools): every bench task's capability names come from it. |
| `catalog.odysseus-a8c147b.json` | The catalog extracted from a live Odysseus import at `a8c147b` (88 tools), the pin the adapter last ran against. |
| `LICENSE` | AGPL-3.0-or-later: the catalogs are extracted from Odysseus (ADR 0002). |

The `odysseus` entry of `conformance/adapter-translation.json` also stays: the frozen trust corpus
(`conformance/TRUST-DIGEST.txt`) pins that file and its `t3_subset`, and changing either would break
the freeze of a closed gate.
