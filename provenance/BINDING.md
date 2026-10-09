# Effect binding, version 1

Status: implemented in `src/binding.rs`, `src/ledger.rs` and `src/lib.rs`; tested in
`tests/effect_binding.rs`. Companion schema stays 0 (ADR 0004, Proposed); the `effect` link gains a
required `binding` field.

**Provenance is evidence, never authority.** A verdict says what retained records say and whether
they agree. It does not allow, deny, authorize or repair anything, and it is never an input to an
execution decision. The only authority over an AIEN effect is the daemon's own check (desk-key MAC,
durable replay claim, grant, intent). The verifier opens the archive read-only (test:
`verifying_reads_and_changes_nothing`).

## What an effect link can bind

| `effect.binding` | Strength | What it binds | What it cannot do |
|---|---|---|---|
| `aien-ledger-slice/1` | strong | request id, trace id, approval id, approved path and bytes, the replay claim, the commit, the grant, the intent and the ack, all from records the AIEN daemon wrote, plus the daemon process and executable | prove that a particular human approved (the MAC proves the desk-key holder did); prove the loaded model authored the proposal (the approved path does not consult the model); authenticate the exported records offline (see Trust) |
| `record_effect_receipt/1` | weaker | tool, outcome, digest of the request arguments, digest of the result data | tell two identical calls apart; name a request or a trace; the producer never writes it on the approved path |

The verdict ends with ` effect=<binding>:strong` or ` effect=<binding>:weak`. A weaker binding is
accepted (it was in v0) and labelled; it is never upgraded.

## Strong binding: `aien-ledger-slice/1`

`COMPANION.json`:

```json
"effect": { "binding": "aien-ledger-slice/1", "approval_binding": "aien.approval.v2",
            "proposal_origin": "model_generation/2" }
```

Retained records (raw files, each named in `records` with path, sha256, bytes):

| Record | Content | Written by |
|---|---|---|
| `ledger_claim` | the replay claim, `approved_submission: "accepted"` | daemon |
| `ledger_committed` | its settlement, `approved_submission: "committed"`, with the compose evidence | daemon |
| `ledger_grant` | the reserved `approved_grant` authorization | daemon |
| `ledger_intent` | `phase: "intent"` | daemon, at the executor's request |
| `ledger_ack` | `phase: "ack"`, `state: "DONE"`, read back from disk by the daemon | daemon |
| `daemon_run` | `{kind: "aien-daemon-run", pid, start_ticks, socket, workspace, executable_sha256, ...}` | the run harness, not the daemon |

Each of the first five is one `ComposeRecordView` exactly as the daemon's `ComposeRecall` command
returns it (`id, cls, kind, subject, tag, links, digest, verified, note, text`); `text` is the
record's JSON as a string. No sovereign-core change is needed to export them.

`proposal_origin` must be stated; silence is `malformed_companion`. It decides which link is missing:

| `proposal_origin` | Meaning | Verdict |
|---|---|---|
| `scripted_turn` | the harness wrote the INTERPLANE model turn | `PASS_LABELLED_INCOMPLETE missing=link:model_turn` (never `PASS complete`) |
| `model_generation/2` | the tool-call text came from the loaded model through AIEN's own generation path, and the DAEMON wrote a record of that generation (sovereign-core #301); `model_turn` and `ledger_generation` are retained | checked (below); nothing missing |
| `model_generation/1` | superseded: the generation was observed and written by the run driver | `FAIL unsupported_binding` |
| anything else | | `FAIL unsupported_binding` |

The loose `record_effect_receipt/1` names no author, so it also leaves `link:model_turn` missing.
The verdict repeats the origin: `effect=aien-ledger-slice/1:strong proposal=model_generation/2`.
A companion that declares `complete` while a link is missing fails with `unlabelled_missing`.

`model_generation/2` checks (`src/modelturn.rs`), after the ledger checks below. The generation record is the
`ComposeRecall` view of the daemon's `effect` note carrying top-level `generation: 1` (spec: sovereign-core
`docs/DAEMON_GENERATION_RECORD.md`). It is refused unless: `verified` is true, the note is `effect`, the four
link slots are zero, the text is exactly one JSON object, `generation == 1` and `v == 1`; its id equals
`model_turn.generation_record_id` (the id the daemon returned in `TurnFinished.generation_record`);
`output_text_sha256` is the SHA-256 of `model_turn.input`; `model_sha256` and `tokenizer_sha256` equal the
exported weights and `tokenizer.json`; `daemon.pid` and `daemon.start_ticks` equal `daemon_run`;
`finish_reason` is `eos` or `max_tokens`, `output_tokens` is positive and the token-id digests are SHA-256 hex;
its id is smaller than the replay claim's id. The real `aien_legacy` dialect parses `model_turn.input` into
exactly one request with the trace's request id, tool, arguments and `source_digest`, sent by the `model` named in
`model_turn.model`. `request_id` and `operation_id` in the record are the client's claims and are not trusted.

Does not prove (from the AIEN document): the digest is of the file read at load time (a swap between hash and load
is not detected); the record is unsigned and a same-user process able to append to the compose ledger could forge
one; the exported `verified` flag is the daemon's statement at export time; no capability or authority; nothing
about correctness of the text.

### Checks (fixed order, first failure wins)

1. `effect.approval_binding` is `aien.approval.v2`, else `FAIL unsupported_binding`.
2. All six records are named in the manifest and retained.
3. Each ledger record has `verified: true`, the expected `note`, a 64-hex `digest`, integer `links`
   and a `text` that parses to a JSON object.
4. Record kinds: claim `accepted`, committed `committed`, grant `approved_grant: 1`, intent
   `phase: intent`, ack `phase: ack`.
5. Ledger order: `claim.id < committed.id < grant.id < intent.id < ack.id`.
6. `committed.claim == claim.id`; committed links the claim; `grant.replay_claim == claim.id`; the
   grant links its promotion, evidence and claim; the commit evidence names the grant's
   `proposal_sha256`, `cx_promotion`, `cx_evidence`.
7. **The approval key is recomputed** from the grant's own nine approval fields in the byte form
   below. It must equal the grant's `approval_key` and the claim's `approval_key`; the claim's
   `approval_id` must equal the grant's.
8. **Request and trace identity.** The grant's and the claim's `trace_id` and `request_id` equal the
   `interplane` link's. Two identical calls differ here: each daemon grant and claim carry their
   own request id, and the ids are inside the MAC-bound approval key.
9. Intent and ack: intent names the grant and repeats its `proposal_sha256`, `path`, `target`,
   `content_sha256`, `prior_sha256`; `tool` is `write_file`; the grant's `target` is `workspace + "/"
   + path`; no ack after the intent is `FAIL effect_interrupted`; ack names the intent and the grant,
   `state` is `DONE` (`NOT_DONE` is `FAIL effect_not_done`, `UNRESOLVED` is `FAIL effect_unresolved`, see
   "Recovery outcomes"), and its `content_sha256` and
   `disk_sha256` equal the grant's `content_sha256`.
10. **Same process.** `claim.executor.{pid,start}` equals `daemon_run.{pid,start_ticks}`;
    `daemon_run.workspace` equals the grant's workspace; `daemon_run.executable_sha256` equals
    `aien.executable_sha256`; the load log's `Binding socket at <path>` equals `daemon_run.socket`.
    With the load-log model and tokenizer digests (already bound to the export), this ties the
    effect to the model the same daemon process loaded.
11. **The INTERPLANE call** (when the trace is retained): the request's tool is `write_file`; its
    `path` equals the grant's; `sha256(content)` equals the grant's `content_sha256`;
    `sha256(approved_proposal_bytes)` equals the grant's `approved_proposal_sha256`;
    `sha256(compose_proposal_bytes)` equals the grant's `proposal_sha256`; the result is `ok` and
    its `data.receipt` names the same grant, intent and ack ids and digests, `DONE`, and the same
    request and trace.

### Exactly which bytes are hashed

Nothing in this binding is JCS. Each digest below has one named form. `src/binding.rs` writes each
form byte for byte; a form it cannot reproduce is refused, never approximated.

`json_string(s)`: an RFC 8259 string with only the mandatory escapes: `"` as `\"`, `\` as `\\`,
U+0008 `\b`, U+000C `\f`, U+000A `\n`, U+000D `\r`, U+0009 `\t`, every other character below U+0020
as `\u00xx` (lowercase hex). U+007F, U+2028 and all non-ASCII are written as raw UTF-8. This is what
`serde_json` writes.

| Digest | Input bytes (UTF-8) | Hash |
|---|---|---|
| record file | the retained file, unchanged (no re-serialization) | SHA-256 |
| `approval_key` (`aien.approval.v2`, sovereign-core `approved_auth::binding_bytes`) | `{"approval_id":S,"approved_proposal_sha256":S,"approver":S,"content_sha256":S,"desk_key_id":S,"path":S,"request_id":S,"trace_id":S,"v":"aien.approval.v2","workspace":S}` with each `S` a `json_string`, these keys in this order (their byte order), no whitespace | SHA-256 |
| `approved_proposal_sha256` | `{"content":S,"path":S}`, compact, that key order | SHA-256 |
| `proposal_sha256` (compose) | the text `filename: <path>\n<content>` | SHA-256 |
| `content_sha256` | the content's UTF-8 bytes | SHA-256 |
| WALDO record pins | Go `json.Marshal` of the record (whitespace-stripped `MarshalIndent`) | SHA-256 |

Test `approval_binding_bytes_are_the_daemons_sorted_compact_object` checks the approval bytes
against a `serde_json` map of the same fields. `fixtures/real-waldo-aien-chain` recomputes the
approval key the real daemon wrote, so producer and verifier are checked against each other byte
for byte on real output.

### Trust, stated plainly

The five ledger files are exports of what the daemon reported. The verifier cannot re-derive a
Cortex record `digest` (it is Omega's record digest) and cannot tell an export from a hand-written
file. What the binding adds over a loose file is **internal consistency across independent
records the daemon writes, and across the INTERPLANE call**: a forger must reproduce a mutually
consistent set including an HMAC-keyed replay key chain, a process identity and the call bytes.
Real assurance that the slice came from the daemon needs the journal itself (re-run
`ComposeRecall` against the live home) or a prefix digest over the whole journal; the verifier
records neither. This is a limit, not a feature.

## Weaker binding: `record_effect_receipt/1`

```json
"effect": { "binding": "record_effect_receipt/1",
            "digest_form": "serde_json.to_vec.sorted-keys/1", "receipt": "effect_receipt" }
```

aien-cli (`crates/aien-cli/src/tools.rs`, `record_effect_receipt`) hashes with
`serde_json::to_vec`. The form `serde_json.to_vec.sorted-keys/1` is: compact JSON, object keys in
UTF-8 byte order, no whitespace, integers in decimal, strings as `json_string`. Differences from JCS
(RFC 8785) that a shared "sorted keys" reading would hide, each pinned by a test:

- key order: JCS sorts by UTF-16 code unit, so U+1F600 sorts before U+FFFF; UTF-8 byte order puts
  U+FFFF first;
- numbers: `1.0` is `1.0` in the producer form and `1` in JCS; a non-integer number is therefore
  **refused** (`canonicalization_mismatch`), because this verifier does not reimplement the
  producer's float printing.

Refusals: `digest_form` absent or different is `canonicalization_mismatch`; a digest that equals the
JCS digest of the same value but not the producer digest is reported as
`canonicalization_mismatch ... is the JCS form`; any other difference is `binding_mismatch`.

The receipt names no request, trace, approval or model. Two requests with the same tool, arguments
and result data produce byte-identical receipts and cannot be told apart (test
`weaker_receipt_cannot_tell_two_identical_calls_apart_and_says_so`). The approved daemon path never
writes it.

## Not covered by `aien-ledger-slice/1`

- The **minted** flow of sovereign-core #296 (`ComposeAuthorize`, `compose_commit`, `minted_grant`).
  The INTERPLANE adapter does not use it; a binding for it would be a new version.
- The approved write path never calls the model ("returns the approved text instead of calling the
  model", `approved.rs`). Authorship is therefore established upstream, in the same trace, by
  `model_generation/2`, not by the approved-write ledger records.
- Model identity inside the approved-write records. They carry no model or tokenizer digests; the
  daemon process identity is the bridge to the generation record (`model_generation/2`).
## Fix-the-test slice: test run, source pin, native claim (VAC M3b)

Status: implemented in `src/testrun.rs`; tested in `tests/fix_the_test.rs` on the committed
`fixtures/synthetic-fix-the-test` (generated by `gen-synthetic <dir> fix-the-test-native`). These
sections ride on top of `aien-ledger-slice/1` and need it: a `test_run` section beside any other
effect binding is `malformed_companion`. They run after the ledger checks and before the
`fixture.class` check.

**The test run is harness evidence, not a daemon effect.** The AIEN daemon only wrote the file
(`bash_eval` is `NOT_EXECUTED`). The run harness executed the task's test command before the
proposal and again after the daemon's ack, and retained what it saw. All of these records are
unsigned exports; the verifier cannot tell a harness export from a hand-written file.

Retained records (named in `records`, raw files):

| Record | Content |
|---|---|
| `task` | `{kind: "vac-task", task_id, repo_commit, test_cmd: [argv...], target_path}`; `target_path` is the one file the task allows to change (`target_file` in the fixture's `TASK.md`) |
| `source_pin` | `{kind: "vac-source-pin", repo, commit, target_path, target_blob_sha256}`: the task's start commit and the SHA-256 of the target file before the fix |
| `test_run_record` | `{kind: "vac-test-run", v: 1, task_id, argv, cwd_rel, exit_code, test_exit_before, stdout_sha256, stderr_sha256, started_unix_ms, duration_ms, tree_commit_after, target_blob_sha256_after}` |
| `test_stdout`, `test_stderr` | the test command's output after the fix, byte for byte |
| `compose_recall` | the daemon's raw `ComposeRecall` report (carries `compose_native`, `omega_sha`) |

`COMPANION.json` gains `"test_run": {"binding": "vac-test-run/1", "task_id": ..., "record": "test_run_record"}`
and, when the daemon reports it, `"aien": {..., "native": {"claimed": bool, "omega_sha": str}}`.

### Checks (fixed order, first failure wins)

1. **`test_run_missing`**: no `test_run` section while the archive names a `task`, `source_pin` or
   `test_run_record` (a bundle that carries a task claims a fix, so dropping the section is not
   silence); or the section's record, or `task`, is absent or not retained.
2. **`unsupported_test_run`**: `test_run.binding` is not `vac-test-run/1`, or the record's `kind`/`v`
   is not `vac-test-run`/`1`.
3. **`task_scope_mismatch`**: `task.target_path` is absent, or differs from the grant's `path`. Without
   it, a run that rewrote the test file instead of the code would still verify. The task record is
   written by the same harness as the bundle, so this binds the declared scope to the write; it does
   not prove the declared scope equals the fixture's `TASK.md` (unsigned-export limit above). Bundles
   captured before this check (no `target_path`) are refused with this code.
4. **`source_pin_mismatch`**: `source_pin.kind` is not `vac-source-pin`; `source_pin.target_path` differs
   from the grant's `path`; `source_pin.target_blob_sha256` differs from the grant's `prior_sha256`
   (null for a new file also fails); `source_pin.commit` differs from `task.repo_commit`.
5. **`test_run_mismatch`**: `task_id` differs between the section, the record and `task`; `argv` differs
   from `task.test_cmd`; `exit_code` is not 0 (the slice claims the test passes after the write);
   `test_exit_before` is 0 (nothing was fixed) or missing; `target_blob_sha256_after` differs from the
   ack's `disk_sha256` (the daemon's read-back) or the grant's `content_sha256`; `stdout_sha256` or
   `stderr_sha256` is not hex, differs from the retained `test_stdout`/`test_stderr`, or the record is absent or labelled not retained (a claimed digest needs the bytes it names).
6. **`native_claim_contradicted`**: see below.

### Native claim, and what a missing one means

`aien.native.claimed: true` is accepted only when the retained `compose_recall` report says
`compose_native: true` and its `omega_sha` (40 hex) equals `aien.native.omega_sha`; with no report, a
report of `false`, or another sha it is `native_claim_contradicted`. `claimed: false` is refused when
the report says native. A malformed `native` object is `malformed_companion`.

In a bundle with a `test_run` section, `link:native` is **missing** (labelled, never complete) unless a
native claim was accepted: the field absent (a daemon older than sovereign-core #346) or `claimed: false`.
The verdict is then `PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native ...`. Bundles without a
`test_run` section are unchanged. `omega_sha` is the build's pinned expected sha, a build-time fact; a
library supplied by override is linked without a sha check (sovereign-core M3a report).

What this check does not do: it reads only the retained `compose_recall` report. The ledger ack records
carry no `compose_native` field, so the ack is not consulted. The recall report is digest-bound but not
cross-checked against the five ledger records, so a producer who writes the bundle can hand-write a
recall report saying `compose_native: true`. That falls under the unsigned-export limit above: the
verifier detects inconsistent or altered evidence, not a producer that fabricates a consistent set.

Does not prove: that the tests are good, that the proposal came from a model (it is scripted:
`link:model_turn` stays missing), that the harness is honest, or that nothing else touched the
workspace between the ack and the second test run.

## Recovery outcomes: effects the ledger does not show as landed (VAC M4d)

Status: implemented in `src/ledger.rs` (`unlanded`); tested in `tests/recovery_verdicts.rs` on the
committed real slice (`real-waldo-aien-chain`) and the synthetic fixtures, mutated and restamped.

**Behaviour before M4d [O, from `tests/effect_binding.rs`]:** an ack with `state: NOT_DONE` was
`FAIL binding_mismatch: ledger_ack.state`; an intent with no ack in the manifest was
`FAIL missing_record_entry: ledger_ack`; `UNRESOLVED` took the same path as `NOT_DONE` [I, same code
line]. None could pass, but all three were indistinguishable from a tampered bundle.

**Now**, in the checks' fixed order (step 9, after the intent and ack are tied to the grant):

| Ledger shows | Verdict | Meaning |
|---|---|---|
| ack `DONE` | as before | the daemon read the file back and it matched |
| ack `NOT_DONE` | `FAIL effect_not_done` | the daemon read the world back and the write did not land |
| ack `UNRESOLVED` | `FAIL effect_unresolved` | the daemon could not tell whether the write landed |
| intent, no `ledger_ack` in the bundle | `FAIL effect_interrupted` | the executor was interrupted after the intent; the write may or may not have landed |
| any other ack state | `FAIL binding_mismatch: ledger_ack.state=<x>` | not a daemon state |

Why FAIL and not `PASS_LABELLED_INCOMPLETE`: a labelled-missing link says "an otherwise valid chain
lacks a link". Here the claimed effect is the link, and the ledger says it is absent or unknown. A PASS
of any kind would let a reader take the effect as having happened. The three codes are checked after the
records before them agree (a forged claim next to a missing ack is reported as the forgery) and before the
INTERPLANE call checks.

**A test run cannot certify a write that did not land.** When the bundle carries a `test_run` section or
any of `task`, `source_pin`, `test_run_record`, each of the three cases above is instead
`FAIL test_run_on_unlanded_effect: <effect code>: <detail>`.

Limits: the verifier does not re-derive that a recovery was correct, only that the exported ack says
what it says. An interrupted run's ack may never be written, and then the bundle is unrecoverable as
evidence of the effect, by design. The NOT_DONE and UNRESOLVED fixtures are synthetic (the real DONE ack
with `state` changed); a bundle exported from a real daemon after an interruption was not produced.

## Independent evaluation: `rsi-eval/2` (VAC M5)

Status: implemented in `src/evaluation.rs`; tested in `tests/evaluation.rs` on the committed
`fixtures/rsi-eval-v2` (a receipt signed by spark-rsi's own code) and on the live M5 bundle by
`adapters/aien/scripts/test-m5-rsi.sh`. Receipt format: spark-rsi `docs/RECEIPT-V2.md`.

The invariant: **no change may be promoted unless its exact identity matches the identity covered by a
valid, independently signed evaluation.** The proposer (the RSI engine) and the judge run as different
operating-system accounts; the judge signs with a key only its account can read. The judge's public
key is a trust input the operator supplies to the verifier (`verify <dir> --judge-key <file>
[--policy-sha256 <hex>]`); it is never read from the bundle, because a bundle that carried its own key could carry the proposer's.

Retained records (raw files, never re-serialized):

| Record | Content |
|---|---|
| `evaluation_receipt` | the judge's version 2 receipt, exactly as the judge printed it |
| `evaluation_policy` | the operator's policy file the judge ran under (`holdout_set_sha256`, `min_holdout_pass_ratio`, `require_admitted`, `allowed_targets`, `protected_paths`) |

`COMPANION.json` gains `"evaluation": {"binding": "rsi-eval/2", "receipt": "evaluation_receipt", "policy": "evaluation_policy"}`.
The section rides on `aien-ledger-slice/1` (with any other effect binding it is `malformed_companion`)
and is checked after the test-run section and before the `fixture.class` check.

### Checks (fixed order, first failure wins)

1. **`evaluation_missing`**: the archive names an evaluation record but has no `evaluation` section; a
   judge key was supplied but the bundle carries no evaluation; or a named record is absent or not
   retained.
2. **`unsupported_evaluation`**: `evaluation.binding` is not `rsi-eval/2`, or `format_version` is
   neither 1 nor 2.
3. **`evaluation_v1_insufficient`**: the receipt is version 1 (or has no `format_version`). Version 1
   does not bind the evaluated change; it stays verifiable as history in spark-rsi but never
   satisfies this check.
4. **`evaluation_malformed`**: a field the digest needs is missing or has the wrong type.
5. **`evaluation_signature_bad`**: the recomputed version 2 digest differs from `receipt_digest` (any
   field edited after signing), or the P-256 signature does not verify under the supplied key (wrong
   or substituted key, a receipt signed by anyone but the pinned judge).
6. **`evaluation_untrusted`**: no judge key was supplied. A receipt is never trusted on its own word.
7. **`evaluation_subject_mismatch`**: `binding.subject_path` differs from the grant's `path`, or
   `binding.subject_sha256` differs from the grant's `content_sha256` or the ack's `disk_sha256`. A
   genuine receipt for different bytes (a substituted change) fails here.
8. **`evaluation_parent_mismatch`**: the receipt's `parent_id` differs from the archive's
   `source_pin.commit`, or the archive has no source pin. A genuine receipt for the same bytes judged
   against another tree fails here.
9. **`evaluation_policy_mismatch`**: `binding.policy_sha256` differs from the SHA-256 of the retained
   policy bytes; the operator pinned a policy (`--policy-sha256`) and the retained policy is not it;
   the written path is under one of the policy's `protected_paths` or not in `allowed_targets`;
   `require_admitted` is not `true`; or `min_holdout_pass_ratio` is not in (0, 1].
10. **`evaluation_holdout_mismatch`**: `binding.holdout_set_sha256` differs from the policy's pinned
    digest.
11. **`evaluation_below_threshold`**: the judge did not admit the change, or
    `holdouts_passed < min_holdout_pass_ratio * holdouts_total`, or the total is 0.

A plain `verify <dir>` without `--judge-key` does not look for an evaluation: a bundle with no
evaluation section passes as an ordinary ledger bundle. An M5 audit must therefore always pass
`--judge-key` (and should pass `--policy-sha256`, the SHA-256 of the operator's own policy file);
with a key supplied, a bundle that carries no evaluation fails `evaluation_missing`.

The same checks run in the harness **before** the write (`evaluation::precheck`, against the exact
bytes about to be proposed), so a change whose receipt does not bind it never reaches the daemon. The
verifier then repeats them against the daemon's own grant and read-back.

### Trust, stated plainly

- The verifier checks that the pinned judge signed this evaluation of these bytes under this policy and
  holdout set. It does not re-run the judge; that the judge's holdouts and layers are good tests is the
  judge's claim.
- The judge refuses to evaluate when the holdout directory is missing, empty, holds anything but
  regular `.json` files, a file does not parse, a suite or case is incomplete, or the set's digest
  differs from the policy's pin. The verifier sees only the digest the judge signed.
- Key separation holds against the proposing account. The operator account can read the judge key
  through sudo; that is the operator's authority, and the M5 receipt lists it as a limit.
