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
`aien-dev/aien-sovereign-core@6554aac7d81b04c91202e6bb0c90d258c8a07f04` (main after PR #204 merged,
which adds single-use approvals on top of the PR #203 authority seam). `aegis` (feature `aegis-gate`, on by default) cannot be a git dependency because
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
drops any minted effect the pipeline did not run. Refusals (`Unknown`, `Mismatch`, `Consumed`,
`Expired`) come back as `denied` with `approval refused: <variant>` and the pipeline denies the
request for good (terminal; the host starts a new request). A spent grant presented again for the
same completed effect is a replay at the adapter: AIEN's ledger returns the existing receipt
(`authorize_and_execute_approved`) and nothing re-executes; through the pipeline a second
continuation is refused first. The stock `write_file` and `bash_eval` descriptors are WORLD_MUTATION,
so they stay pending until continued. `with_effects(name, bits)` re-declares a stock capability's
`ToolEffects` (used by tests with a LOCAL_EPHEMERAL temp-dir write and an EXTERNAL_IRREVERSIBLE
deny); AIEN's authority decides from the new bits.

**Spend point (limit).** AIEN spends the grant when it mints the effect, inside `present_approval`,
not when the provider runs it, and aien-mcp at 6554aac has no way to give a spent grant back. So the
grant is gone before the pipeline's own checks (digest, expiry string, catalog) and before the
provider call. Outcomes, all tested in `tests/approval.rs`:

| What fails after the mint | What happens | Second effect? |
|---|---|---|
| the pipeline refuses the continuation (wrong digest, stale clock, catalog change) | `denied` reported; the minted effect is dropped (`discard_unexecuted`); the grant stays `Consumed` | no: nothing ran |
| the provider rejects the call (`CallOutcome::Rejected`, e.g. missing directory) | `error` reported; AIEN removes the ledger entry; the grant stays `Consumed`, the request is terminal | no: the host recovers with a new request and a new grant after fixing the cause |
| the provider outcome is unknown (`Uncertain` or a wire error) | `error` reported; AIEN's ledger keeps `Uncertain`; the same effect (same idempotency key) is `ReconciliationRequired` for any later grant (`broker.rs` `execute_effect`) | no, but it needs reconciliation by the host; not exercised by a test because the in-process provider cannot return `Uncertain` |

What AIEN cannot do: reserve a grant and commit it only when the effect runs, so a refusal between
mint and execute burns an approval that bought nothing, and the human approves again. That costs a
re-approval, never a duplicate effect. Recorded as a limit; a sovereign-core issue is drafted in the
PR body (not filed). Other limits: grants and the minted handles live in memory only (restart: no
pending request, fail closed); the minted `approval.expires_at` is left empty, so expiry is AIEN's
(`now >= expires_at` against the epoch the host passes), and the pipeline's string clock is a
second, separate check; the desk is separated from other desks by handle discipline, not by type.

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

Successful reads carry `content_kind: workspace_content`, `trust: workspace_untrusted`.
`with_trusted_workspace(true)` switches to `trusted_runtime`. Non-executed outcomes carry null
content kind and trust, as CORE.md requires.

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
