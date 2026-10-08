# interplane-adapter-aien

Reference adapter that lets the INTERPLANE `Pipeline` (Crossveil) talk to AIEN's existing
authority machinery. License: AGPL-3.0-or-later (see `LICENSE`). Design: `docs/adr/0003`.

**This adapter never changes AIEN and AIEN keeps all authority.** It builds nothing that AIEN
could mistake for its own decision: no `AuthorizedEffect`, no `DoctrineDecision`, no
`SafetyDecision`. `authorized` appears only when AIEN's own gate returned `Ok` for a read, or when AIEN's
`EffectLane::authorize` minted an `AuthorizedEffect` after AIEN's `EffectClassAuthority` said Allow.
`requires_approval` and `denied` carry AIEN's reason. The approval id on `requires_approval` is an adapter handle with no authority: grants come only from AIEN's `ApprovalDesk` and are
spent only by AIEN. Nothing under `rust/`, `spec/`, `python/` or
`conformance/` and no AIEN repository is modified.

## Pinned AIEN sources

`aien-capability` and `aien-mcp` are git dependencies pinned to
`aien-dev/aien-sovereign-core@9b5e6e82359f4ad6b6f03042e83ef6411c5f15ce` (main 2026-10-08; `crates/aien-capability` and `crates/aien-mcp` are byte-identical to 4d4dfd4, `git diff --stat 4d4dfd4 9b5e6e8 -- crates/aien-capability crates/aien-mcp` is empty, and the live gate below passed against a daemon built from 9b5e6e8. 4d4dfd4 was main 2026-10-07, after PR #296 merged: the daemon honours only grants it minted itself and refuses every caller-written authorization note, sovereign-core #261; and PR #295: an approved proposal carries a signed requirements goal, a missing one is refused, #289. `crates/aien-capability` and `crates/aien-mcp` are byte-identical to 8bcd79b, main after PR #262 merged: the authenticated `ComposeApprovedProposal` daemon command the ledger route uses; `crates/aien-capability` and `crates/aien-mcp` are byte-identical to 7d37da1, after PR #260 merged: two-phase approval spend, a grant is reserved at mint, committed when the effect runs, released if the effect is dropped, and `EffectLane::with_clock` re-checks expiry at commit; before #260 `crates/aien-capability` and `crates/aien-mcp` are byte-identical to 0bdc97a, main after PR #208 merged, which binds the idempotency ledger to effect identity and lets an approver revoke an unspent grant; before it, PR #207 merged,
which makes the authority exposure-aware, on top of PR #204 single-use approvals and the PR #203
authority seam). `aegis` (feature `aegis-gate`, on by default) cannot be a git dependency because
`aegis-runtime`'s own `Cargo.toml` reaches a sibling checkout by relative path
(`../aien-protocols/crates/*`). The build therefore expects this layout next to the INTERPLANE
checkout:

```
<parent>/interplane/adapters/aien           (this crate)
<parent>/interplane-audit/aegis-runtime      aien-dev/aegis-runtime  @ AEGIS_RUNTIME_REV in PINS
<parent>/interplane-audit/aien-protocols     aien-dev/aien-protocols @ AIEN_PROTOCOLS_REV in PINS
```

All sibling pins live in one file, `PINS` (this directory). CI (`.github/workflows/ci.yml`, job `adapter-aien`)
and local setup both call `setup-siblings.sh`, which reads `PINS`, so they use the same commits:

```
cd <parent>/interplane
adapters/aien/setup-siblings.sh          # clones/checks out ../interplane-audit/{aegis-runtime,aien-protocols}
cd adapters/aien && cargo test           # toolchain is pinned by /rust-toolchain.toml (1.99.0)
```

The script also fails if the `aien-sovereign-core` rev in `Cargo.toml` differs from `SOVEREIGN_CORE_REV` in `PINS`.
To move a pin, change `PINS` (and `Cargo.toml` for sovereign-core) in the same PR.
Without the `aegis-gate` feature the crate still builds and every `decide()` is `denied` (fail closed).
Building `aegis-runtime` writes `mojo/libaegis_simd.so` inside that checkout (its `build.rs`).

## Reused from AIEN (called for real)

All paths relative to the pinned clones.

| What | Where |
|---|---|
| `ToolDescriptor::new`, `capability_digest` | `aien-sovereign-core/crates/aien-capability/src/tool.rs:7,30` |
| `catalog_digest` (the catalog digest we publish) | `aien-capability/src/tool.rs:43` |
| `routing_class`, `speculation_safe` (effect classification) | `aien-capability/src/effects.rs:54,74` |
| `SessionManager::enroll`, `speculative_lane` | `aien-mcp/src/session.rs:38,29` (wraps `McpBroker::admit`, `broker.rs:53`) |
| `SpeculativeLane::invoke_speculative` (read execution; refuses unsafe effects) | `aien-mcp/src/broker.rs:151` |
| `SpeculativeLane::stage_effect_intent` (records an intent, executes nothing) | `aien-mcp/src/broker.rs:181` |
| `MemoryWire` (in-process `McpWire`) | `aien-mcp/src/memory.rs:11` |
| `aegis::pre_dispatch_check` (live allowlist gate) | `aegis-runtime/src/enforcement.rs:31` (`ALLOWED_SKILLS` at `:14`) |

`AuthorizedEffect` is never constructed here. The only production mint is crate-private inside
`aien-mcp`, reached through `EffectLane::authorize(intent, scope, &dyn EffectAuthority)`; the adapter
passes AIEN's own `EffectClassAuthority`. A test (`adapter_source_never_constructs_authorized_effect`)
greps `src/lib.rs` for struct-literal construction, `authorize_for_test`, `mint(` and any local
`EffectAuthority` impl.

## Effect path (0.2)

Reads are unchanged (speculative lane). Everything else (`routing_class` not Pure/ReadOnly) goes:
stage intent (a speculation-safe local-ephemeral tool cannot be staged by AIEN, so a plain
`EffectIntent` is used; it carries no authority) -> `EffectLane::authorize` with
`EffectClassAuthority` -> in `decide`:

| AIEN decision | Crossveil decision | Executes |
|---|---|---|
| Allow / AllowRestricted (PURE, READ_*, LOCAL_EPHEMERAL) | `authorized`; the minted effect is held until `execute` | yes, via `EffectLane::execute_effect`; result carries the `EffectReceipt` |
| RequireApproval (EXTERNAL_WRITE, WORLD_MUTATION, SECRET_BEARING, SPAWN_PROCESS) | `requires_approval`, `runtime_state` has `staged=true` and `intent_digest=...` | no |
| Deny (EXTERNAL_IRREVERSIBLE, unknown tool or bits, stale intent) | `denied` with AIEN's reason | no |
| Contain | `denied` with AIEN's reason | no |

Approvals (aien-mcp single-use grants, PR #204), routed through the host-only continuation API
(0.3 cut A3, `spec/CORE.md` "Approval continuation"). `decide` never reads a grant: a
`requires_approval` answer stays pending and carries `approval.approval_id`, a handle this adapter
mints (`aien-approval:<request_id>:<intent digest prefix>`) so the host can correlate the
continuation; the handle carries no authority (the authority is the grant AIEN's desk issues, whose
id is private to aien-mcp). Nothing in `arguments`, `extensions`, the envelope or model text is read
as a grant. The host's approver calls `AienAuthority::issue_approval(req, expires_at)`, which issues
a grant through AIEN's `ApprovalDesk` bound to that exact effect, then
`AienAuthority::present_approval(req, &grant, now)`, which spends it through
`EffectLane::authorize_approved` and returns the runtime's continuation decision (citing the minted
id). That decision goes to `Pipeline::continue_approval`. Because the pipeline borrows its runtime
for its whole life, hosts wrap the adapter in `AienShared` (give the pipeline a clone) and call
`AienShared::continue_approval(&mut pipeline, trace, request_id, &grant, now_epoch, now)`, which
reads the request and digest from the pipeline's own pending entry, runs the continuation, and
drops any minted effect the pipeline did not run. Refusals (`Unknown`, `Mismatch`, `Consumed`, `Reserved`,
`Expired`) come back as `denied` with `approval refused: <variant>` and the pipeline denies the
request for good (terminal; the host starts a new request). A spent grant presented again for the
same completed effect is a replay at the adapter: AIEN's ledger returns the existing receipt
(`authorize_and_execute_approved`) and nothing re-executes; through the pipeline a second
continuation is refused first. The stock `write_file` and `bash_eval` descriptors are WORLD_MUTATION,
so they stay pending until continued. `with_effects(name, bits)` re-declares a stock capability's
`ToolEffects` (used by tests with a LOCAL_EPHEMERAL temp-dir write and an EXTERNAL_IRREVERSIBLE
deny); AIEN's authority decides from the new bits.

**Spend point (limit).** AIEN spends the grant when it mints the effect, inside `present_approval`,
not when the provider runs it. aien-mcp at 0bdc97a can revoke only an unspent grant (#208), so a spent grant still cannot be given back (sovereign-core #206). So the
grant is gone before the pipeline's own checks (digest, expiry string, catalog) and before the
provider call. Outcomes, all tested in `tests/approval.rs`:

| What fails after the mint | What happens | Second effect? |
|---|---|---|
| the pipeline refuses the continuation (wrong digest, stale clock, catalog change) | `denied` reported; the minted effect is dropped (`discard_unexecuted`); the grant is RELEASED (sovereign-core #260: reserved at mint, never spent here; AIEN records the reason, a later mint re-checks expiry, revocation and binding) | no: nothing ran |
| the provider rejects the call (`CallOutcome::Rejected`, e.g. missing directory) | `error` reported; AIEN removes the ledger entry; the grant stays `Consumed` (spent at commit, just before the provider call), the request is terminal | no: the host recovers with a new request and a new grant after fixing the cause |
| the provider outcome is unknown (`Uncertain` or a wire error) | `error` reported; AIEN's ledger keeps `Uncertain`; the same effect (same idempotency key) is `ReconciliationRequired` for any later grant (`broker.rs` `execute_effect`) | no, but it needs reconciliation by the host; not exercised by a test because the in-process provider cannot return `Uncertain` |

What AIEN cannot do: reserve a grant and commit it only when the effect runs, so a refusal between
mint and execute burns an approval that bought nothing, and the human approves again. That costs a
re-approval, never a duplicate effect. Recorded as a limit; a sovereign-core issue is drafted in the
PR body (not filed). Other limits: grants and the minted handles live in memory only (restart: no
pending request, fail closed); the minted `approval.expires_at` is left empty, so expiry is AIEN's
(`now >= expires_at` against the epoch the host passes), and the pipeline's string clock is a
second, separate check; the desk is separated from other desks by handle discipline, not by type.

## Not the production effect path

The adapter reaches AIEN through `aien-mcp` (`EffectLane`, `EffectClassAuthority`, `ApprovalDesk`,
in-memory single-use grants). At the pinned rev no other sovereign-core crate depends on
`aien-mcp` or `aien-capability`. The production effect path is `aien compose` (`aien-cli`
`src/compose.rs`): a CLI step talks to the `aien-runtime` daemon, which runs omega COMPOSITION-2
through `aien-omega-compose` (FFI to `librx_compose.a`, pinned by `omega.lock`); the approval is a
durable Cortex `authorization` record, the write is bracketed by durable `effect` intent/ack
records (`aien-runtime/src/effects.rs`), and every step leaves an `aien-cli`
`record_effect_receipt` file. None of that is reached by the default adapter, so adapter
conformance proves the `aien-mcp` authority path; the opt-in `ComposeLedgerAuthority` below is
the route that reaches the compose/World/Cortex boundary.

## Durable effect ledger (`ComposeLedgerAuthority`, opt-in)

`src/compose_ledger.rs` wraps `AienShared` (approval desk, `decide` and reads unchanged) and routes
an approved `write_file` through a running `aien-cli daemon` (sovereign-core #249) instead of the
in-process provider. The route opens only when AIEN's `EffectLane` minted the effect for that
request (`holds_minted_effect`); that in-process effect is then dropped unexecuted. An approved
write has exactly one route:

1. **Authenticated handoff.** `ComposeApprovedProposal` with an `ApprovedProposal` (trace id,
   request id, approval id, approver, path, content, approved `{content, path}` digest, content
   digest) and `approval_mac`, HMAC-SHA256 under the approval desk key (`DeskKey`, made by
   `aien compose desk-key --create 1` in the daemon's compose home) over the canonical binding.
   The daemon re-checks the MAC and claims the request id, approval id and approval key durably;
   any replay (same request, same approval id, after a restart, after a crash) is refused.
2. **Compose.** The approved text runs through the daemon's compose path: J-Space branch, AEGIS
   verify, World commit, Cortex promotion and evidence records.
3. **Grant.** The daemon itself writes the grant: a reserved `approved_grant` authorization keyed
   on the returned `compose_proposal_sha256`, confined to the workspace, linked to the promotion,
   evidence and replay claim. The adapter reads it back and checks every field; it never writes a
   grant, so there is no client-minted fallback.
4. **Intent, write, ack.** `ComposeEffectIntent` (stop, revoke, spent, stale, confinement and the
   grant's committed backing checked by the daemon), the write (tmp + rename inside the
   workspace), `ComposeEffectAck` (the daemon reads the disk: DONE / NOT_DONE / UNRESOLVED).

The desk key never reaches the model: `ComposeLedgerAuthority::new` refuses a key inside the
workspace (or a workspace inside the key's directory) and a key file that is a symlink, not owned
by this user, group- or world-readable, or malformed; only its `desk_key_id` is recorded. The MAC
proves that the holder of the desk key approved these exact fields, not that a particular human
did; the OS user is the outer boundary. The socket client speaks the daemon's JSON line protocol
directly, so `aien-runtime` (whose build needs the omega libraries) is not a dependency.

Boundary, stated on every receipt (`LEDGER_BOUNDARY`): authenticated daemon handoff -> compose
verify + AEGIS -> J-Space branch -> World commit -> Cortex promotion/evidence -> daemon grant on
compose_proposal_sha256 -> effect intent -> write -> ack. It is the production effect path; it is
not WALDO provenance and not a full provenance chain: trace_id and request_id are on the handoff, the replay claim and the daemon grant, but not yet on the effect intent, the ack or `record_effect_receipt` (#76).
Evidence: `tests/compose_ledger.rs` rows 1-8 (approved write with both hash identities, forged
approval id, stale target, replay including a re-presented approval id, operator stop/resume,
adapter restart, the T4 injection subset, forged handoffs) need `AIEN_BIN` (an `aien-cli` binary
built from sovereign-core main 9b5e6e8) and `cargo test -- --ignored`; rows 10 and 11 (#260 expiry and re-mint) likewise; row 9 (desk key out of the model's
reach, key file rules) always runs.

Recorded run at this pin (CPU evidence only, 2026-10-08 00:52Z to 00:58Z): `aien-cli` built from
sovereign-core 4d4dfd4 with the compose engine linked and the GPU engine not linked (build shows
`has_omega_compose`, not `has_omega_gpu`; the daemon logs `Backend: NativeTransformerBackend/CPU-reference`),
sha256 `279d38992b72f7c3f888d9b02c5ac7b26e57f6a47746c4e8107b2a9081ae4056`, model Llama-3.2-1B-Instruct
on the CPU reference backend. `tests/compose_ledger.rs` rows 1-8, 10, 11: 10 passed; `tests/compose_ledger_attacks.rs`
x1-x4: 4 passed. This is not GPU evidence and not native-OS evidence. A daemon built with the GPU engine
linked selects the GB10 backend whenever the chip is present, so build without `AIEN_OMEGA_DIR` for CPU runs.

Since sovereign-core #260 (in 8bcd79b and the pinned 4d4dfd4): the effect lane is built `with_clock(host_clock())`
(epoch seconds; `AienAuthority::set_clock` for tests), a grant presented while reserved is
refused as `Reserved`, and a dropped effect releases its grant. On the ledger route the adapter
holds the minted effect (and its reservation) while the daemon runs, checks the grant's expiry
with the same clock just before the handoff (row 10: expired between grant and commit, refused,
no handoff, no write), and releases the reservation afterwards: the daemon's durable replay
ledger is what spends the approval. Row 11: the released grant re-minted with the same approval
id is answered by the daemon with the original result (ALREADY_COMMITTED, no second compose) and
refused by the adapter, with no second write.

Since sovereign-core #295 (in the pinned 4d4dfd4): every approved proposal the adapter hands the
daemon carries `requirements` and `requirements_mac`. The host approves the exact bytes of a
`write_file`, so there is no natural-language goal whose requirements the daemon could check; the
adapter binds the explicit "none" (`BOUND_REQUIREMENTS`, the empty goal), MACed with the desk key
under tag `aien.requirements.v1` over the same approval binding. A proposal without it is refused
(`RequirementsUnbound`, #289). Since #296 (#261): the daemon honours only grants it minted itself;
`tests/compose_ledger_attacks.rs` x1 and x3 present a caller-written authorization note and assert
the daemon refuses it (`sovereign-core #261`), the ledger gains no record and no effect is written.

## Reimplemented, and why

- **The workspace read provider** (`read_file`, `list_dir`) behind `MemoryWire`. AIEN ships no
  filesystem provider in `aien-mcp`; ADR 0003 names an in-process `LocalToolServer`, but that
  needs an `rmcp` transport pair for no gain here, so the simpler `McpWire` implementation
  `MemoryWire` is used. The call still goes through the real `SpeculativeLane`.
- **Effect enrollment.** No AIEN crate enrolls a catalog (`CURRENT_STATE.md`: nothing depends on
  `aien-capability`), so the adapter enrolls four descriptors named after aegis `ALLOWED_SKILLS`
  (`read_file`, `list_dir`: `READ_FILESYSTEM`; `write_file`: `WORLD_MUTATION`; `bash_eval`:
  `SPAWN_PROCESS | WORLD_MUTATION`) using AIEN's own `ToolEffects` bits.
- **Argument-schema check** (`missing required argument: <key>`, `argument <key> must be string`),
  because AIEN has no schema validator on this path; messages match the CORE.md mock wording.
- **Workspace confinement** (paths must stay under the workspace root). An adapter-side
  restriction that can only deny. It stands in for `aien-cli::SafetyEngine::evaluate`.

## Not possible: `SafetyEngine`

ADR 0003 asks for `SafetyEngine::evaluate` before `pre_dispatch_check`. It cannot be linked: the
`aien-cli` crate declares `mod safety;` privately in `main.rs` and its `lib.rs` is empty, so
there is no public path without editing AIEN. This is a discrepancy with the ADR; the adapter
uses the aegis gate plus confinement instead, and the decision's `authority.policy_engine`
names the gate actually used. Reaching `SafetyEngine` needs an AIEN change (export it) or an
operator-side process.

## Not executed

`bash_eval` has no provider body, and any call whose decision is not authorized
returns `status: error`, `code: execution_error`, message `not executed by the reference adapter`
if `execute()` is ever called on it. `write_file` executes only with an AIEN-minted effect.

## Fail closed

`AienAuthority::unavailable()` and a build without the `aegis-gate` feature make `decide()`
return `denied` with `aien runtime not available` for every request. Never `authorized`.

## Catalog digests

`catalog()` publishes AIEN's `catalog_digest` (SHA-256 over the name-sorted per-tool digests,
tool digest = SHA-256 of name, effect bits little-endian u32, schema digest) as
`sha256:<hex>`. INTERPLANE's `Catalog::compute_digest` uses a different formula (JCS over
name/schema pairs), so the two differ. Both are recorded, unequal, in `catalog.extensions.aien`
(`catalog_digest`, `interplane_computed_digest`, `digests_equal`). `mapping_table(true)` pins to
the published value, so `stale_capability` protection reuses AIEN's digest.

## Provenance

Results are labelled by what they are (0.3 cut E4, CROSSVEIL.md rule 7):

| Capability | `content_kind` | `trust` |
|---|---|---|
| `read_file` | workspace_content | workspace_untrusted |
| `list_dir` | tool_result | workspace_untrusted |
| `write_file` (after approval) | tool_result | trusted_runtime (AIEN's receipt plus the adapter's acknowledgement) |
| anything else | tool_result | external_untrusted |

`with_trusted_workspace(true)` lifts the two reads to `trusted_runtime`. Non-executed outcomes carry
null content kind and trust, as CORE.md requires.

## Exposure (0.3 cut E4)

`decide` hands the pipeline's `CallContext.exposure` to AIEN (`EffectLane::with_exposure`,
sovereign-core #207) in AIEN's own `Exposure` / `TrustLevel` types, and keeps it per request so the
grant presented later is checked against what the model saw when it asked. AIEN's
`EffectClassAuthority` holds every effect class for approval when the floor is below `user_supplied`
or exposure is absent; its reason then names exposure. AIEN already required approval for those
effect classes, so the visible change is the reason and the explicit rule, not a new block. A host
registers the user's request first; with an empty ledger the floor is `external_untrusted`.
Evidence: `tests/exposure.rs` (5 host-registered untrusted sources and a workspace read, against
`write_file` and `bash_eval`: all held, 0 executed, nothing written). A `bash_eval` outside the
catalogued forms is refused earlier by the AEGIS gate.

## Live verified run gate

`scripts/test-live-daemon.sh` is the one command that ties the live daemon rows to the offline
provenance verifier:

```
AIEN_BIN=<native aien-cli> AIEN_LEDGER_MODEL_DIR=<dir with model.safetensors, tokenizer.json, config.json> \
  OUT=<fresh dir> scripts/test-live-daemon.sh        # LIVE_ROWS="row1_ x1_ x2_ x3_ x4_" limits the rows
```

It (1) runs the `#[ignore]`d live tests against a real `aien-cli daemon` (`--ignored --test-threads=1`;
each row starts a daemon that loads the model, about 6.5 min on the CPU reference backend with a debug
build) and fails if zero tests ran or row 1 or an attack test `x1_`..`x4_` did not; (2) row 1 exports its
run with `provenance_export::export_bundle` (the daemon's own `ComposeRecall` views of the claim,
settlement, grant, intent and ack, the load log, the trace, the loaded model files, `COMPANION.json`)
and the gate runs `provenance verify` on it; the only accepted verdict is exactly
`PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn`;
(3) it tampers with hard-linked copies of the bundle and requires named refusals: an untouched copy
still passes (negative control), a changed proposal content or a changed ack fails `digest_mismatch`,
the same change with the manifest restamped fails `binding_mismatch`, and a truncated record fails
`size_mismatch`; (4) it writes `OUT/receipt.json` (adapter commit, binary sha256, sovereign-core
revision, omega lock, verdict, test names, negative-check verdicts, timestamps).

What a pass proves: for one real daemon run, the claim, settlement, grant, intent and ack the daemon
wrote agree with each other and with the INTERPLANE call (path, bytes, request, trace, the approval key
recomputed from the grant's own fields, the daemon process identity), and the verifier refuses the
tampered copies above.

What it does not prove: the exported records carry no signatures, so the verifier cannot tell an export
from a hand-written file (`provenance/BINDING.md`, Trust); the model turn is scripted, so the verdict is
labelled incomplete (no authorship claim); the desk MAC for `ComposeAuthorize` stays default-off (not
changed here; this path authenticates by the desk-key MAC regardless); CPU reference backend only; the
daemon (before sovereign-core #346) does not report whether the native compose library is linked or which revision it was
built from, so the receipt records those as caller-asserted or `UNVERIFIED`; a daemon with #346 reports
`compose_native` and `omega_sha`, which the fix-the-test slice below carries and checks.

Recorded run: `evidence/live-gate-2026-10-08/receipt.json` (CPU, debug `aien-cli` sha256 9704f29f..., sovereign-core 9b5e6e8 caller-asserted, omega 6c6180c). It ran `LIVE_ROWS="row1_ x1_ x2_ x3_ x4_"` (5 tests); rows 2-8, 10 and 11 passed against the same binary in an earlier unrecorded-by-this-gate run (14 of 14), not by this receipt.

## Fix-the-test slice

`scripts/test-fix-the-test.sh` (also run at the end of `test-live-daemon.sh` unless `SKIP_SLICE=1`) is the
first product-shaped run on top of the live gate: an agent step fixes a deliberately failing test, and the
whole run is verified offline.

```
AIEN_BIN=<aien-cli> OUT=<fresh dir> scripts/test-fix-the-test.sh      # one live row, about 8.5 min on CPU
```

The fixture `fixtures/fix_the_test/` is a tiny C project: `clamp()` returns `lo` where it must return `hi`,
so `make test` fails before the fix (GNU make exits 2 on a failing recipe; the gate requires non-zero) and
exits 0 after. `TASK.md` carries the task id and the exact test command; `solution/clamp.c` is harness
material (not copied into the task workspace). No binaries are committed.

The live row `slice1_fix_the_test_lands_and_tests_pass` (`tests/compose_ledger.rs`, `#[ignore]`d) copies the
fixture to a workspace, `git init`s and commits it, runs `make test` (expected red), then drives a
SCRIPTED `write_file` proposal whose content is the corrected `src/clamp.c` through the existing adapter and
the daemon's `ComposeApprovedProposal` path (desk-key MAC, durable claim, grant, intent, ack), waits for
`DONE`, and runs `make test` again (expected green). `src/fix_the_test.rs` builds the task, source-pin and
`vac-test-run/1` records; the exporter retains them next to the daemon's ledger records, and the daemon's
raw `ComposeRecall` report, which since sovereign-core #346 says whether the compose library is native.
The gate then runs `provenance verify` and, on hard-linked copies, requires a named refusal for a changed
exit code, a swapped stdout digest, a wrong target blob, a dropped test run, a tampered source pin and a
forged native claim (`provenance/BINDING.md`, "Fix-the-test slice").

Expected verdict: `PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong
proposal=scripted_turn`; with a daemon that does not report `compose_native` it is `missing=link:model_turn,link:native`
(labelled, never complete). Never `PASS complete`: the model turn is scripted.

What it proves: the approval is bound to the exact bytes written; the write went through the daemon's
ledger; the tests were run after the write and their output is retained and digest-checked; the source
pin matches the grant's `prior_sha256` (the bytes the daemon replaced are the bytes of the pinned commit's
file); the bundle's native claim agrees with the daemon's own report.

What it does not prove: the model turn is scripted (no authorship claim); the records are unsigned exports,
so the verifier cannot tell an export from a hand-written file; the test run is harness evidence, not a
daemon effect (the daemon only writes, `bash_eval` stays `NOT_EXECUTED`); nothing ties the second test run
to the workspace state beyond the digest of the target file and the harness's own commit; the desk MAC for
`ComposeAuthorize` stays default-off (not changed here); CPU reference backend only.

Recorded run: `evidence/fix-the-test-2026-10-08/` (receipt and the small bundle files; the 2.4 GB weights are
hard links in the original and are not copied).

## Reproducible demo

`scripts/demo-verified-fix.sh` is the one command that runs the verified fix-the-test workflow end to end and
writes a machine-made record, `$OUT/DEMO-RESULT.md`: exact revisions (interplane, sovereign-core, omega,
physics and aienos locks, daemon hash, model and tokenizer hashes, host, rustc), whether the daemon says it is
native (its own `Compose:` line and `ComposeRecall`), the starting task, the approved write (path, content
digest, approval binding id), the ledger ack, the test results before and after, the verifier verdict and the
tamper summary, plus "What is proven" and "What is not". It runs `scripts/test-fix-the-test.sh` once.

Prerequisites: `git jq make cc cargo rustc sha256sum`, the sibling checkouts from `adapters/aien/setup-siblings.sh` (run it once from the interplane checkout first, otherwise the gate build fails with "failed to read .../interplane-audit/aegis-runtime/Cargo.toml"), the Llama-3.2-1B-Instruct snapshot (`AIEN_LEDGER_MODEL_DIR`, default the
unsloth snapshot in the Hugging Face cache), a clean interplane checkout, and a native sovereign-core daemon.

```
# with a native aien-cli you already have
AIEN_BIN=<native aien-cli> SOVEREIGN_CORE_REV=<its sovereign-core rev> OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh

# or build it from the four checkouts (each HEAD must equal its lock; CPU, debug build)
SC_DIR=<aien-sovereign-core> OMEGA_DIR=<omega @ SC_DIR/omega.lock> PHYSICS_DIR=<physics @ OMEGA_DIR/physics.lock> \
AIENOS_REPO=<aienos @ OMEGA_DIR/aienos.lock> OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh
```

The demo exits non-zero if the daemon reports a stub compose library, if `compose_native` and the `Compose:`
line disagree, if the model or tokenizer hash in the daemon log differs from the file on disk, if the verdict
is anything but the labelled-incomplete one (`missing=link:model_turn`; a scripted run is never `PASS
complete`), or if any tamper is not refused. The stub check reads the daemon's report after the gate's run
(about 9 to 15 minutes), not before it. Recorded run: `evidence/demo-2026-10-08/DEMO-RESULT.md`. Nothing here
changes a security default.

## Bounded self-improvement, independently judged (VAC M5)

`scripts/test-m5-rsi.sh` runs one bounded self-improvement cycle where the proposing agent cannot forge
the evidence that approves its own change:

```
scripts/m5-setup-judge.sh <pinned public key file>        # once: the judge account makes its own key
AIEN_BIN=<aien-cli> OUT=<fresh dir> SPARK_RSI_DIR=<spark-rsi target/release> \
M5_JUDGE_KEY=<pinned public key file> scripts/test-m5-rsi.sh
```

`m5-setup-judge.sh` refuses to finish unless it can run a command as the proposing account and that
account gets "Permission denied" reading the judge key, and unless neither service account can write the
pinned public key, its directory or any directory above it, or use sudo.

Three separate parties, three separate operating-system accounts:

- **Proposer** (`aien-rsi`, no sudo): `spark-rsi propose` reads a read-only copy of the task workspace
  (`fixtures/rsi_dashes/`, whose tests fail while its README contains dashes) and prints one proposal. The
  hook (`scripts/m5-propose-and-judge.sh`) refuses a workspace with uncommitted changes or symbolic
  links, and a proposal for any file other than `README.md`.
- **Judge** (`aien-judge`, home mode 700, holds the only copy of `judge.key`): `spark-rsi-judge` receives
  its own copies of the parent tree, the candidate tree, the holdout suite (`m5/holdouts/`) and the
  operator policy (`m5/policy.json`). Before running anything it refuses a holdout set that does not hash
  to the policy's pinned digest, a subject outside `allowed_targets` or under `protected_paths`, and a
  candidate that differs from the parent in any file but the subject. It builds both trees and runs their
  own `cargo check` and `cargo test` inside a bubblewrap sandbox (no network, no environment, only the
  tree and the toolchain visible, so a build script or test in the candidate cannot read the key), runs the holdouts, re-hashes the subject and signs a
  version 2 receipt (`spark-rsi` `docs/RECEIPT-V2.md`) naming the parent commit it compared against.
- **Operator path** (the harness and the daemon): the harness refuses a judged policy that is not byte for
  byte its own `m5/policy.json`, then `evaluation::precheck` checks the receipt against the pinned public
  key, the exact bytes about to be proposed, the task's commit and the policy pin; only then does the
  scripted proposal go through `ComposeApprovedProposal` with the normal approval, exactly like the
  fix-the-test slice. After the tests pass, a second approved write restores the original README
  (rollback) and the tests fail again.

The bundle carries the receipt and the policy as records with an `evaluation` section, and
`provenance verify <bundle> --judge-key <file> --policy-sha256 <sha of m5/policy.json>` checks it offline
(`provenance/BINDING.md`, "Independent evaluation"). Without the key the verdict is
`FAIL evaluation_untrusted`: a bundle never carries the key that verifies it. The gate then requires a
named refusal for each counterfeit: an edited score, layer or subject digest (`evaluation_signature_bad`),
an edited policy (`evaluation_policy_mismatch`), the wrong pinned key (`evaluation_signature_bad`), a
dropped section (`evaluation_missing`), a genuine receipt for a different README
(`evaluation_subject_mismatch`), a receipt signed with a throwaway key, a version 1 receipt
(`evaluation_v1_insufficient`), a policy with a lower bar re-signed into a consistent pair
(`evaluation_policy_mismatch` against the operator's pin), a workspace with uncommitted changes (the hook
refuses), and holdout suites that are missing, corrupt, shrunk or carry a stray file (the judge refuses to
sign).

What it proves: the change that landed is byte for byte the change the judge evaluated, judged against the
task's own commit; the judge ran under the operator's pinned policy and holdout set and signed with a key
the proposer cannot read; a version 1 receipt, a substituted score or a substituted change is refused
before the write and again offline; the change can be undone through the same approved path.

What it does not prove: the operator account can still read the judge key through sudo (that is the
operator's authority, not the agent's); the holdouts are not secret from the proposer (its account can
read the operator's files), only fixed by digest; the proposal is a deterministic rule-based edit, not a
model turn, so the verdict stays labelled incomplete (`missing=link:model_turn`); the speed check allows
200% slowdown because timing a microsecond program here varied by up to 79% between identical builds; the
desk MAC for `ComposeAuthorize` stays default-off (not changed here).

Recorded run: `evidence/m5-rsi-2026-10-08/` (`receipt.json`: gate PASS on adapter 0c8a92d and spark-rsi 4becebb, tests
failing before, passing after and failing again after the rollback; `judge-receipt.json`: the judge's signed
version 2 receipt, 4 of 4 holdout suites; `negative-verdicts.txt`: the control and 18 named refusals;
`speed-noise.txt`: the timing noise behind the 200% allowance). The same receipt is the cross-language
fixture in `provenance/fixtures/rsi-eval-v2/`.

## Build and test

```
cd adapters/aien
cargo build --offline
cargo test --offline
cargo fmt --check && cargo clippy --offline --all-targets -- -D warnings
```

Building `aegis` runs its `build.rs`, which writes `mojo/libaegis_simd.so` into the pinned
aegis clone as an untracked file. This crate is its own Cargo workspace and is not a member of
`rust/Cargo.toml`.
