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
"effect": { "binding": "aien-ledger-slice/1", "approval_binding": "aien.approval.v2" }
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
   + path`; ack names the intent and the grant, `state` is `DONE`, and its `content_sha256` and
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
against a `serde_json` map of the same fields. The first real fixture additionally recomputes the
approval key the real daemon wrote (see `fixtures/real-waldo-aien-chain`).

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
- Who authored the proposal. On the approved path the daemon "returns the approved text instead of
  calling the model" (`approved.rs`), so the loaded model is **not consulted**. A model-authored
  proposal needs the daemon's own proposer path and has no binding here.
- Model identity inside the daemon records. They do not carry model or tokenizer digests; the
  process identity above is the bridge. A daemon-written model digest is a sovereign-core cut (see
  the PR description).
