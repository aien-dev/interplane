# Provenance companion manifest v0 (schema 0)

NOT A MASTER PLAN. The smallest offline-checkable bridge from training-data evidence to a runtime
effect. Outside INTERPLANE Core (ADR 0004): Core does not read or depend on it. Provenance is
evidence, never authority: nothing here decides, grants or authorizes anything.

## What it binds, link by link

`COMPANION.json` sits beside retained, immutable records and names them by path, raw-byte
SHA-256 and size. It reuses identities the records already carry; it mints no ids.

| Link | Records | Identity reused | Checked |
|---|---|---|---|
| corpus + license assertions | `waldo_run_bom` (corpus BOM embedded) | WALDO `corpus_bom_sha256` | WALDO digest of the embedded `corpus_bom`; `corpus_license_assertions` equals its `licenses` keys (restated, never judged) |
| training plan / run | `waldo_plan`, `waldo_run_bom`, `waldo_run`, `waldo_preflight`, `waldo_model_bom`, `run_weights` | WALDO `plan_sha256` / model id, `runs[].bom_sha256`, `RUN.json bom_sha256`, preflight raw SHA-256 | every pin; run `complete` and not simulated; `current_run_id`; backend equals `execution.backend` |
| export (a conversion is its own record) | `release_bom`, `export_weights`, `export_config`, `export_tokenizer`, `export_chat_template` or explicit `null` | WALDO release BOM `source_bom_sha256`, `artifacts[]` | source pin, run and model id, every exported file; Hugging Face: safetensors tensor data equals the run's pinned weights |
| load support | `export_config`, `export_tokenizer` | none (a check result) | `LlamaForCausalLM` + Hugging Face `tokenizer.json` with `config.vocab_size` = tokenizer size; computed result must equal `load_support.declared` |
| AIEN candidate + loaded model | `candidate_manifest`, `aien_load_log` | CandidateManifestV1 `id`, `[executables]`, `[model]`; aien-cli daemon line `model_sha256=` / `tokenizer_sha256=` | all equal the exported bytes; refused unless load support is `supported_structural` |
| INTERPLANE trace | `interplane_trace` (array of envelopes) | `trace_id`, `request_id` | every envelope passes `interplane_core::validate_envelope_value`; same trace; request and its result present |
| effect | `ledger_claim`, `ledger_committed`, `ledger_grant`, `ledger_intent`, `ledger_ack`, `daemon_run` (strong, `aien-ledger-slice/1`) or `effect_receipt` (weaker, `record_effect_receipt/1`) | the daemon-written ledger records; or aien-cli `record_effect_receipt` v1 | see `BINDING.md`: request, trace, approval key recomputed, path and bytes, replay claim, grant, intent, ack, daemon process; the weaker receipt binds tool and digests only and is labelled `weak` |

WALDO record digests are SHA-256 of Go `json.Marshal`; the files are `json.MarshalIndent` of the
same value, so the verifier strips insignificant whitespace from the retained bytes and hashes
that (`src/gojson.rs`). Checked on the real fixture: run BOM, model BOM, plan and embedded corpus
BOM all reproduce WALDO's own pins.

## Completeness and missing lineage

Every link is an object or an explicit `null`; a record may be `"retained": false` (it keeps its
`sha256` and, for WALDO JSON, `waldo_sha256`, so the pins that cite it are still checked).
`completeness.state` is `complete` only when nothing is missing; otherwise `incomplete` with
`missing` listing exactly the `link:<name>` and `record:<name>` entries that are absent. A model
with no training evidence uses `lineage: {"kind": "unknown_pretraining", "origin": {...}}`; it
may not carry WALDO records. Nothing is filled in to make a chain look whole.

## Verdicts (one line, fixed check order, first failure wins)

`PASS complete` (only when no link is missing) | `PASS_LABELLED_INCOMPLETE missing=<sorted,comma,list>` | `FAIL <code>: <detail>`.
Codes: `missing_companion`, `unparseable_companion`, `unsupported_companion`,
`malformed_companion`, `missing_record`, `missing_record_entry`, `size_mismatch` (partial or
truncated write), `digest_mismatch`, `unlabelled_missing`, `completeness_mismatch`,
`unparseable_record`, `binding_mismatch`, `run_not_complete`, `lineage_conflict`,
`conversion_mismatch`, `incompatible`, `load_support_mismatch`, `load_unsupported`,
`invalid_envelope`, `not_in_trace`.

```
cargo run -q -- verify fixtures/synthetic-full-chain     # PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn
cargo run -q -- verify fixtures/real-waldo-aien-chain    # PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2
cargo run -q -- verify fixtures/synthetic-weak-receipt   # PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=record_effect_receipt/1:weak
cargo run -q -- verify fixtures/real-waldo-smoke         # PASS_LABELLED_INCOMPLETE missing=link:aien,link:effect,link:interplane
cargo run -q -- verify fixtures/real-waldo-tokenizer-json  # PASS_LABELLED_INCOMPLETE missing=link:effect,link:interplane
```

`seal` writes a new `COMPANION.json` from a skeleton and refuses to overwrite one: a sealed
record is never rewritten to match a new result.

## Fixtures

- `fixtures/real-waldo-smoke`: REAL bytes, unmodified, from a WALDO CPU smoke run (openwaldo/waldo
  tree 0fd421a, PyTorch backend, model `pytorch-smoke`, run `80b1d6cbe8ca88a1`, 2026-10-06).
  Corpus, plan, run, model BOM and Hugging Face export verify. Its tokenizer is
  `OpenWALDOByteTokenizer` custom code, not a Hugging Face `tokenizer.json`, so the load-support
  check says `unsupported_tokenizer` and the AIEN, trace and effect links are labelled missing.
- `fixtures/real-waldo-tokenizer-json`: REAL bytes. WALDO CPU smoke run exported by the patched exporter
  (aien-dev/waldo `export-tokenizer-json` 09f262c, writes a Hugging Face `tokenizer.json`), then
  really loaded by aien-cli (sovereign-core PR #248 head 8e7b14d, debug build, CPU). The `aien` link
  is present with `candidate_id: null` (no frozen candidate, no `candidate_manifest` record); the
  daemon log is retained in full, including its warm-up refusal (plain base model, no chat
  template). Effect and INTERPLANE links are labelled missing.
- `fixtures/real-waldo-aien-chain`: REAL bytes from ONE run (CPU evidence), see `REAL-CHAIN.md`. A tiny
  WALDO `chatml-v1` model, supervised-fine-tuned on tool-call conversations we wrote and exported with
  `tokenizer.json` and the standard ChatML template (fork aien-dev/waldo `export-tokenizer-json` 4c407d7), is
  loaded by the AIEN daemon (aien-cli, sovereign-core main 12c1a5f, release build, CPU-reference backend).
  AIEN's own generation path (`StreamTurn`) on that loaded model produces the tool-call text; the INTERPLANE
  `aien_legacy` dialect parses it; the adapter drives one approved `write_file`; the daemon's ledger records
  and the trace are retained, together with the DAEMON'S OWN record of the generation (sovereign-core #301,
  `TurnFinished.generation_record`). The verifier re-parses the text, binds it to the trace and the ledger, and
  checks the daemon's record: output digest = digest of the turn text, model and tokenizer digests = the exported
  files, same daemon process, written before the effect. Verdict `PASS complete`: every link is verified from the
  retained bytes of this one run. Residual limits: digests are of the files read at load time (a swap between hash
  and load is not detected), the record is unsigned (a same-user process that can append to the compose ledger
  could forge one), no authority follows. The model is a tiny memoriser with a fixed prompt: no capability claim.
- `fixtures/synthetic-weak-receipt`: SYNTHETIC, the same chain with the weaker v1 receipt (also labelled `link:model_turn` missing).
- `fixtures/synthetic-full-chain`: SYNTHETIC, generated by `src/synth.rs` (reproduced byte for
  byte by a test). Schema-faithful WALDO subset plus one labelled deviation (an HF
  `tokenizer.json` in the export), a CandidateManifestV1-shaped `CAND-SYNTH-0`, an aien-cli load
  line, two INTERPLANE requests and one effect receipt. No AIEN process loaded it.

## Limits

- Strong binding (`aien-ledger-slice/1`) ties the effect to the daemon's ledger records and to the
  model only through process identity (the claim's executor pid and start time, the daemon
  executable digest, the socket path in the load log). The daemon's records carry no model or
  tokenizer digest; a daemon-written model digest is a sovereign-core change (see `REAL-CHAIN.md`).
- The ledger files are exports. The verifier cannot re-derive a Cortex record digest or tell an
  export from a hand-written file; assurance beyond internal consistency needs the live journal or
  a journal prefix digest, which are not recorded. See `BINDING.md`, Trust.
- A verdict says `complete` only when the model that produced the effect's content is linked to the
  effect. A scripted model turn (`proposal_origin: scripted_turn`) and the weaker receipt always leave
  `link:model_turn` missing. `model_generation/2` needs the daemon's own generation record and checks it (text digest,
  model and tokenizer digests, daemon process, ordering); a missing, malformed or mismatched record is a `FAIL`.
  `PASS complete` therefore exists only for chains where every link is verified from retained bytes of one run. It
  is evidence about provenance, never permission, and says nothing about the model's capability.
- The weaker `record_effect_receipt` v1 names no request or trace; identical calls cannot be told
  apart. It is accepted and labelled `weak`. Its digest form is `serde_json.to_vec.sorted-keys/1`,
  not JCS; a mismatch is `canonicalization_mismatch` (`BINDING.md`).
- The minted flow of sovereign-core #296 (`ComposeAuthorize`, `compose_commit`, `minted_grant`) has
  no binding yet; the INTERPLANE adapter uses the approved path.
- `training_code` and `conversion_tool` are recorded assertions (`checked: false`); WALDO's
  release BOM does not carry the tool identity.
- Load support is structural plus one observed behaviour: the daemon's warm-up and one generation ran on
  the tiny model (CPU). The model is a memoriser with a fixed prompt; nothing here measures capability.
- WALDO's own `index verify` / `model export` checks are not re-run here; this is a consumer.
- Provenance never grants execution. Nothing in this crate is consulted by AIEN or INTERPLANE.
