# The first real chain (CPU evidence)

Fixture: `fixtures/real-waldo-aien-chain`.
Verdict: `PASS_LABELLED_INCOMPLETE missing=link:daemon_generation_record effect=aien-ledger-slice/1:strong proposal=model_generation/1`.
It is labelled incomplete on purpose and cannot say `PASS complete` (see "What is missing"). Provenance is
evidence, never authority: it grants and authorizes nothing, and no AIEN or INTERPLANE code reads it.

## The rule the verdict follows

A chain is complete only when training, this model, the AIEN load, an INTERPLANE trace and an effect all come
from the same model and execution. So:

- `proposal_origin: scripted_turn` (the harness wrote the model turn) always yields
  `PASS_LABELLED_INCOMPLETE missing=link:model_turn`, never `PASS complete`. The weaker
  `record_effect_receipt/1` names no author, so it also leaves `link:model_turn` missing. Declaring either
  complete is refused (`unlabelled_missing`).
- `proposal_origin: model_generation/1` is checked as far as the bytes allow and still leaves
  `link:daemon_generation_record` missing, because the daemon writes no record of a generation.
- Today no effect-bearing chain can reach `PASS complete`. That needs the daemon cut below.

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
   4d4dfd459ae7e7c6c175edc5017816d079c621a9, #295 and #296 merged, RELEASE build) on that export, writes
   `daemon-run.json` (pid, start ticks, socket, executable digest), then:
   a. sends one `StreamTurn` (the path `aien-cli chat` uses) with the prompt "Save the meeting summary to
      summary.txt." at temperature 0, and keeps the reply as `generation.json`;
   b. feeds that exact text to the INTERPLANE `aien_legacy` dialect (`model-turn.json`), which yields one
      `write_file` request;
   c. drives approval and the write through the INTERPLANE AIEN adapter; the daemon writes its ledger records.
3. Retained unchanged: daemon log, the five `ComposeRecall` views, `daemon-run.json`, `generation.json`,
   `model-turn.json`, and the INTERPLANE trace (request, requires_approval result, ok result).

How the GPU engine stayed OFF: the release binary was built with `AIEN_OMEGA_DIR` unset (CPU stub of
`aien-omega-gpu`) and `AIEN_GB10_QWEN3_DECLARED_ATTEMPT` unset; `ldd` shows no CUDA or NVIDIA library; the daemon
log says `NativeTransformerBackend/CPU-reference (Omega GPU engine not linked, explicit fallback)`; the driver
removes `AIEN_GPU_BACKEND`, `AIEN_REQUIRE_BLACKWELL` and `AIEN_OMEGA_DIR` from the daemon environment.

Adapter used: interplane main 0ff60f2 plus the UNCOMMITTED worktree diff of the coordinator's branch
`ipx/issue-78-repin` (read-only snapshot, `run-inputs/adapter-pr78-worktree-diff.patch`, sha256
a11d8ac03f3683baedb187d4d504203419f9a736512597fccab8ed0ba3e47e86), because main is refused by a 4d4dfd4 daemon
(`RequirementsUnbound`). Replace this with the merged interplane#78 commit when it lands.

## What the verifier checks for `model_generation/1`

- The text in `model_turn` is byte for byte the generation's `output_text`, whose digest is `output_sha256`.
- The real `aien_legacy` dialect (the verifier links `interplane-lenshift`) parses that text into exactly one
  request with the trace's request id, tool, arguments and `source_digest`, and the trace names the same model as
  the sender.
- The generation names the model digest the daemon load log carries; that digest is already bound to the exported
  weights, and the export descends from the retained WALDO run.
- The generation and the ledger name the same daemon process (pid and start ticks), the process that wrote the
  claim, grant, intent and ack.
- Everything the ledger binding already checks (`BINDING.md`): request, trace, approval key recomputed, path and
  bytes, claim, commit, grant, intent, ack.

## What is missing (labelled, not filled)

- `link:daemon_generation_record`. AIEN's generation path (`StreamTurn`) wrote no record: the journal held 5
  records before and after (`generation.json`). So "this text came from the model with this digest" is asserted
  by the run driver (`written_by: run-driver`), tied to the daemon only by process identity. A record written
  by the daemon would close it; `written_by: daemon` is refused (`unsupported_binding`) until such a record has
  a defined shape.
- Ledger files are exports, not the live journal; no journal prefix digest (`BINDING.md`, Trust).
- The model is a memoriser; the prompt is fixed; temperature 0. This shows the plumbing, not capability.
- Not the minted flow, not GPU, not a frozen candidate (`candidate_id` is null), adapter not yet at a merged commit.
- The INTERPLANE trace timestamps are the driver's wall clock; the adapter's approval clock is the harness epoch.

## The minimal sovereign-core cut (not made here)

When the daemon serves `StreamTurn` (and when `RunComposeTask` generates a proposal), append one record through
the same journal writer, class `effect`, kind `generation`, with fields: `model_sha256`, `tokenizer_sha256`
(the digests it logged at load), `prompt_sha256` (of the rendered prompt), `output_sha256` (of the returned text),
`total_tokens`, `executor {pid, start}`, and the request or trace id when the caller supplies one. Return the
record id in `TurnFinished`. Then the verifier can require that record and the chain can reach `PASS complete`.
