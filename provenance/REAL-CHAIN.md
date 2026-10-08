# The first real chain (CPU evidence)

Fixture: `fixtures/real-waldo-aien-chain`.
Verdict: `PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2 candidate=none`.
`complete` here means exactly this: every link from the training corpus to the effect is present and was
verified from the retained bytes of ONE run (CPU evidence). It does not mean the model is capable, the
weights are good, or anyone authorized anything. Provenance is evidence, never authority: it grants and
authorizes nothing, and no AIEN or INTERPLANE code reads it.

## The rule the verdict follows

A chain is complete only when training, this model, the AIEN load, an INTERPLANE trace and an effect all come
from the same model and execution. So:

- `proposal_origin: scripted_turn` (the harness wrote the model turn) always yields
  `PASS_LABELLED_INCOMPLETE missing=link:model_turn`, never `PASS complete`. The weaker
  `record_effect_receipt/1` names no author, so it also leaves `link:model_turn` missing. Declaring either
  complete is refused (`unlabelled_missing`).
- `proposal_origin: model_generation/2` needs the DAEMON'S OWN generation record (sovereign-core #301,
  `docs/DAEMON_GENERATION_RECORD.md`) and checks it as below. With it, nothing is missing.
- `model_generation/1` (a record written by the run driver, the first version of this fixture) is superseded and
  refused (`unsupported_binding`).
- A missing, malformed, altered or mismatched generation record is a refusal (`FAIL`), never a pass. The daemon
  itself says that an absent record is an absent claim.

## Prerequisite analysis

1. Chat template. The AIEN loader (`aien-inference-abi`, `tokenizer.rs`) accepts a template only if its text
   equals a known one. The WALDO `chatml-v1` export used a whitespace-controlled spelling, so it was not
   recognised. Fix on the fork (aien-dev/waldo `export-tokenizer-json`, commit
   4c407d7f5ed33feaf234dd8ebed34ac197cfc0ef): ship the exact Hugging Face ChatML text; a Go test pins it. The
   template is recorded in the export (`chat_template.jinja`, `tokenizer_config.json`) and in the manifest.
2. KV pool. The daemon sizes its KV pool from the model context and needs the 643-token warm-up prompt plus a
   64-block watermark. Context 1024 gives 64 blocks, so nothing can be admitted: an earlier draft of this
   document called the resulting 120 s warm-up timeout "inference that was too slow". That was wrong; it was
   KV exhaustion (`runtime step error: KV pool exhausted`, seen in the daemon's stderr). With context 2048
   (128 blocks) the warm-up really runs: `Warm-up: 1 token in 81 ms over 643 prompt tokens`.
3. Model capability. The model must emit a tool call in a dialect INTERPLANE parses. The 1-layer byte model
   of the first attempt could not. The retained model (2 layers, width 128, bf16, context 2048) was trained
   by supervised fine-tuning (WALDO structured conversations, `chatml-v1`, assistant turns supervised) on 46
   short conversations we wrote (`run-inputs/conversations.jsonl`, CC0), each ending in one `write_file`
   call in the `aien_legacy` text form. It reproduces the memorised call for the fixed prompt, greedy and
   repeatable (checked twice). It is a memoriser, not a general model; that is stated, not hidden.
4. The approved write path does not call the model. The content comes from the model UPSTREAM in the same
   trace, and the adapter writes it.

## What was run (one execution)

1. `run-inputs/train-and-export.sh`: WALDO (fork 4c407d7) trains `toolcall-tiny` on `run-inputs/conversations.jsonl`
   (PyTorch CPU, 90 epochs, 921 steps; log in `run-inputs/train.log`) and exports Hugging Face with `tokenizer.json`.
2. `driver/main.rs` (a scratch crate, kept as evidence) starts `aien-cli daemon` (sovereign-core main
   12c1a5ff18b70e748b678280c4210788f820c712, sc#301 merged, RELEASE build) on that export, writes
   `daemon-run.json` (pid, start ticks, socket, executable digest), then:
   a. sends one `StreamTurn` (the path `aien-cli chat` uses) with the prompt "Save the meeting summary to
      summary.txt." at temperature 0 and reads `TurnFinished.generation_record` (id 6). It exports that record
      with the daemon's own `ComposeRecall` (`ledger-generation.json`) and keeps the reply text in `model-turn.json`
      together with the id;
   b. feeds that exact text to the INTERPLANE `aien_legacy` dialect, which yields one `write_file` request;
   c. drives approval and the write through the INTERPLANE AIEN adapter; the daemon writes its ledger records
      (claim 7 ... ack 22).
### The corpus generator (kept outside the repository)

`conversations.jsonl` (46 conversations, bytes retained in `run-inputs/`) was written by a scratch script,
`gen_corpus.py`. This repository takes no Python outside `python/`, so the script is not in the branch. Its exact
bytes are kept here by name and digest, and in a durable directory outside the repo:

- file: `gen_corpus.py`, 1908 bytes, sha256 `cae40fda46f5a876c0b93dee9bc0dadd5323b5f6f891f73fc3130720e7c9adcb`
- durable path: `~/workspace/evidence/interplane-76/2026-10-07/train2/gen_corpus.py` (the whole run evidence is under
  `~/workspace/evidence/interplane-76/2026-10-07/`: training dir, run records, driver, build log, PIDs, mutation logs)
- it generated `conversations.jsonl`; no verifier check and no `COMPANION.json` digest reads the script, so the verdict
  does not depend on it. The corpus bytes the training used are the retained `conversations.jsonl`.

3. Retained unchanged: daemon log and stderr, the generation record, the five `ComposeRecall` views,
   `daemon-run.json`, `model-turn.json`, and the INTERPLANE trace (request, requires_approval result, ok result).
   Process ids of this run: daemon 1256385 (killed by the driver at the end). An earlier failed attempt on a build
   that lacked the compose library left daemon 1214846, killed by that PID alone.

Build and how the GPU engine stayed OFF: `cargo build --release --locked -p aien-cli` at 12c1a5f with
`AIEN_OMEGA_DIR` unset (so `aien-omega-gpu` is the CPU stub) and `AIEN_OMEGA_COMPOSE_DIR` pointing at the pinned
checkout 01f6a746 (the compose library, CPU C code, which the daemon needs to open the ledger at all; a stub build
writes no generation record). A private `CARGO_TARGET_DIR`; `ldd` shows no CUDA or NVIDIA library; the daemon
log says `NativeTransformerBackend/CPU-reference (Omega GPU engine not linked, explicit fallback)`; the driver
removes `AIEN_GPU_BACKEND`, `AIEN_REQUIRE_BLACKWELL` and `AIEN_OMEGA_DIR` from the daemon environment.

Adapter used: the MERGED INTERPLANE adapter at interplane main c982fd3 (interplane#81, re-pinned to sovereign-core
4d4dfd4, signs the requirements goal). No snapshot or uncommitted diff is involved. Run id `real-run-04` (trace
`real-run-04`, daemon pid 1641935, killed by the driver). The earlier run `real-run-03` (adapter snapshot of the
uncommitted #78 worktree) is replaced: it only differed in the adapter, and its records stay in the evidence dir.

## What the verifier checks for `model_generation/2`

The generation record (`records/aien/ledger-generation.json`) is located by the id the daemon returned in
`TurnFinished` (kept in `model-turn.json`), and parsed strictly:

- it is a `ComposeRecall` view with `verified: true`, note `effect`, all four link slots zero, and a text that is
  exactly one JSON object with top-level `generation == 1` and `v == 1`;
- `output_text_sha256` equals the SHA-256 of the exact text in `model_turn`, which the real `aien_legacy` dialect
  (the verifier links `interplane-lenshift`) parses into exactly one request with the trace's request id, tool,
  arguments and `source_digest`; the trace names the same model as the sender;
- `model_sha256` equals the exported weights and `tokenizer_sha256` the exported `tokenizer.json`; the load log
  line carries the same digests, and the export descends from the retained WALDO run;
- `daemon.pid` and `daemon.start_ticks` equal the daemon that wrote the ledger (`daemon-run.json`), the process
  that wrote the claim, grant, intent and ack;
- `finish_reason` is `eos` or `max_tokens`, `output_tokens` is positive, the two token-id digests are SHA-256 hex;
- the record's id is smaller than the replay claim's id: the generation precedes the effect.

Everything the ledger binding already checks still runs (`BINDING.md`): request, trace, approval key recomputed,
path and bytes, claim, commit, grant, intent, ack. `request_id` and `operation_id` in the record are the client's
claims and are recorded, never trusted (a test pins that changing them changes nothing).

## In plain words

Proves: for this one run, the bytes of the training corpus, the exported model, the daemon's load, the text the
daemon generated, the tool call the adapter parsed from it and the write the daemon ledgered all agree, by digest
and process identity. Does not prove: anything about GPU or the native OS (CPU only), model quality (tiny
memoriser), authority (provenance grants none), signing (the generation record is unsigned), or the caller-asserted
`request_id`/`operation_id`. Residual gaps are listed next.

## What this does NOT prove (residual limits)

From the AIEN document, repeated here so the verdict is not read as more than it is:

- The digests are of the files the daemon read at load time. The loaders then open the same paths again, so a swap
  between the hash and the load is not detected. Nor that the file on disk still has that digest later.
- The record is not signed. A process of the same user that can append to the compose ledger could append a record
  that passes the per-record digest. The daemon refuses to write one through the socket (`ComposeNote`), but the file
  itself is not protected against that user.
- The `verified` flag and `digest` in the exported view are the daemon's statement at export time; this verifier does
  not recompute the engine's record digest. Ledger files are exports, not the live journal; there is no journal prefix
  digest (`BINDING.md`, Trust).
- Not that the text is correct, safe, or what an operator approved. No capability, permission or authority.
- The model is a tiny memoriser with a fixed prompt at temperature 0. This shows the plumbing, NOT capability.
- CPU evidence only; not a frozen candidate (`candidate_id` is null); not the minted flow; the trace timestamps are the driver's wall clock and the adapter's approval clock is the harness epoch.
- run-04 evidence (this fixture): ~/workspace/evidence/interplane-76/2026-10-08/ (SHA256SUMS sha256 474176b5f6b66a3672e30211f67d1b5cb0754d460c1bdf4102387f5e4a181600)
