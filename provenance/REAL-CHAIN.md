# The first real chain (CPU evidence)

Fixture: `fixtures/real-waldo-aien-chain`. Verdict: `PASS complete effect=aien-ledger-slice/1:strong proposal=scripted_turn`.
Provenance is evidence, never authority: it grants and authorizes nothing, and no AIEN or INTERPLANE
code reads it.

## Prerequisite analysis (written before the run)

Question: can a tiny model we train load in the AIEN daemon so that the daemon really runs it?

1. Chat template. The AIEN loader (`aien-inference-abi`, `tokenizer.rs`) accepts a chat template only if
   its text equals a known one (`CHATML_PLAIN`, the SmolLM2 form, Qwen3, Llama 3). No template means
   "plain model": warm-up is refused and no inference runs (the previous real fixture). WALDO's
   `chatml-v1` template was a whitespace-controlled spelling of the same output, so the loader would
   not recognise it. Fix on the fork (aien-dev/waldo `export-tokenizer-json`, commit
   4c407d7f5ed33feaf234dd8ebed34ac197cfc0ef): ship the exact Hugging Face text; a Go test pins it; both
   spellings render identical bytes on 6 cases (checked with jinja2) and the new text equals
   `CHATML_PLAIN` byte for byte. The template is recorded in the export (`chat_template.jinja`,
   `tokenizer_config.json`) and in the manifest (`export_chat_template`, checked against the release BOM).
2. Context. The daemon's warm-up prompt is 643 byte-tokens, so a model with context 128 is refused
   (observed). The retained model has context 1024.
3. Model capability. A 1-layer, 32-wide byte model trained for 10 steps on 9.5 KB of text we wrote (our corpus repeated 20 times)
   cannot follow instructions. The daemon runs it (observed: the warm-up proposal hit the daemon's 120 s
   limit and the log says so), but it cannot author a useful proposal. This is a limit of the model, not
   a hidden fault.
4. The approved path does not consult the model (sovereign-core `approved.rs`: the approved text is
   returned instead of calling the model). So even a capable model would not change the proposal
   on this path. The chain therefore says `proposal_origin: scripted_turn`, and `model_turn` is refused.

## What was run

Everything under `fixtures/real-waldo-aien-chain/records` is from one execution:

1. `run-inputs/train-and-export.sh`: WALDO (fork 4c407d7) trains `prov-tiny` on `run-inputs/corpus.txt`
   (PyTorch CPU) and exports Hugging Face with `tokenizer.json`.
2. `driver/main.rs` (not part of the crate): starts `aien-cli daemon` (sovereign-core main
   4528c64954b4dc49a712dcee743fc6028e3b8b51, #296 merged, debug build, CPU-reference backend, GPU engine
   not linked) on that export, writes `daemon-run.json` (pid, start ticks, socket, executable digest),
   then drives one `write_file` through the merged INTERPLANE adapter (interplane main 0ff60f2, #77)
   exactly like row 1 of `adapters/aien/tests/compose_ledger.rs`: propose, host approval, continue.
3. Retained unchanged: the daemon log, the five `ComposeRecall` views (claim, committed, grant,
   intent, ack), `daemon-run.json`, and the INTERPLANE trace (request, requires_approval result, ok result).

## What it proves

- The five ledger records the daemon wrote agree with one another and with the call: request id, trace
  id, approval id, path, content digest and both proposal digests.
- The approval key the real daemon wrote equals the key recomputed from its nine fields in the byte form
  of `BINDING.md`: producer and verifier agree byte for byte on real output.
- The effect was executed by the daemon process that loaded this export (executor pid and start time,
  executable digest, socket path in the load log) and that export descends from the retained WALDO run.
- Altered records, identity, trace or weights are refused (`tests/real_chain.rs`).

## What it does not prove (labelled, not filled)

- The loaded model did not author the proposal. The INTERPLANE model turn is scripted (`source` id
  `scripted`); the adapter clock is pinned to the synthetic epoch the adapter harness uses.
- The daemon's records carry no model or tokenizer digest, so model to effect is linked by process
  identity, not by a daemon-written digest.
- Ledger files are exports: no journal prefix digest, no live re-read (see `BINDING.md`, Trust).
- Not the minted flow (#296 `ComposeAuthorize`), not GPU, not a frozen candidate (`candidate_id` is null).
- The tiny model's warm-up timed out; its ability is untested.

## Minimal cuts that would close the gaps (not made here)

- sovereign-core: write the loaded `model_sha256` and `tokenizer_sha256` into the replay-claim record
  (or a `ComposeRecall`-readable runtime record). Then the model to effect link needs no process bridge.
- Model-authored proposal: the adapter must use the minted flow (`RunComposeTask` then `compose_commit`
  then `ComposeAuthorize`) and a binding version for it. Blocked on sc#295 (requirement enforcement) and the
  adapter update the coordinator owns (interplane#78).
