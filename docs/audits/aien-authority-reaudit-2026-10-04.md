# AIEN authority re-audit, 2026-10-04 (INTERPLANE 0.2 lane L2)

Read-only audit. No AIEN code changed. Every claim is from `git show origin/main:<path>` or a
command run on 2026-10-04.

## Commits audited (origin/main, fetched 2026-10-04)

| Repo | 0.1 pin | origin/main now | Commit date | Moved? |
|---|---|---|---|---|
| aien-sovereign-core | 7580039 | **0bcd558ed9adc3aafcc59526fba6417a86fe804b** | 2026-10-04 15:44 -05:00 | yes, 3 commits (#200 AttentionGeometry ABI, #201 attention wiring tests, #202 omega-gpu pin). `git diff 7580039 origin/main -- crates/aien-mcp crates/aien-capability crates/aien-cli/src/safety.rs` is empty: no authority-relevant change |
| aegis-runtime | f4e8709 | **f4e8709** (2026-10-04 13:21 -05:00) | | no |
| aien-protocols | 3a4cdbe | **3a4cdbe** (2026-10-04 13:22 -05:00) | | no |
| aien-architecture | 6c95697 | **6c95697** (2026-10-04 14:22 -05:00) | | no |

Note: the 0.1 pins for aegis-runtime, aien-protocols and aien-architecture are still current main.
Only sovereign-core moved, and not in anything this audit is about. Local `~/workspace` checkouts
were stale or ahead (sovereign-core c83eb5b, aegis-runtime e8eed5a, protocols 1554330); they were
not used as evidence.

## The nine answers

1. **Canonical loop: declared, and it is neither a keeper.** aien-architecture
   `docs/plans/CURRENT_CODE_TO_R0_R16_MIGRATION.md:34,146` calls aegis-runtime `agent.rs:130`
   "LLM agent loop ... Legacy orchestration; never on the reaction path; Oracle -> retire (R16)".
   ADR 0016 scope note (`docs/adr/0016-resident-reaction-architecture.md:1485-1493`): the
   `aegis-runtime` program "is legacy orchestration ... becomes the reference oracle ... R8 does
   not build on it"; line 1588: do not create independent `aegis-runtime/` schedulers.
   Loop B (aien-cli REPL) has no ADR or plan row naming it canonical or retired (only its
   `goals.rs` and `commands.rs:1536` are marked Replace, lines 138-139). `CONTEXT.md:14` merely
   lists "native chat, the AEGIS agent loop, and subagents" as inference clients. Verdict: loop A
   is declared legacy/oracle; loop B is **undeclared**; the intended canonical loop is the
   resident reaction runtime (ADR 0016), which is not Rust and not yet built in these repos.
2. **Canonical authority: none of the three.** Migration doc line 147 says aegis-runtime
   `enforcement.rs:31` + `policy_guard.rs` are "Not authority. Effect admission belongs to
   capability root + effect broker (ADR 0005)", decision **Replace** (R7/R8). ADR 0016
   `:54,:359,:1504`: the capability mint and validator live in the trusted root, natively the
   AIENOS kernel capability authority (aienos `native/capability/`, C); "only the AIENOS root
   mints the grant"; AEGIS "decides no policy" (`:56`). `docs/04-capabilities-skills-tools-mcp.md:
   109-122`: `EffectIntent<T> -> policy evaluation -> AuthorizedEffect<T>`, "production authority
   must be minted only by the policy path", no unrestricted production constructor. So the
   intended canon is capability root + Effect Broker (ADR 0005); `SafetyEngine`,
   `pre_dispatch_check` and `ExecutionAuthority` are interim. `DoctrineDecision` is not named
   canonical anywhere I found.
3. **Yes, still private.** `aien-sovereign-core` `crates/aien-cli/src/main.rs:14` `mod safety;`
   (not `pub`); `crates/aien-cli/src/lib.rs` is two comment lines (no modules); the crate is a bin.
   `SafetyEngine` is `pub struct` only inside a private module (`safety.rs:40`), used at
   `tools.rs:76`.
4. **No implementer.** `git grep ExecutionAuthority origin/main` in all three repos: only
   aegis-runtime `src/execution/broker.rs:16-17`, re-exported `src/lib.rs:21`. Signature:
   `#[async_trait] pub trait ExecutionAuthority: Send + Sync { async fn authorize(&self, request:
   &ActionRequest) -> Result<DoctrineDecision, String>; }` where `ActionRequest` is the local
   `{session_id, run_id, capability, intent, target, params: Value}` (`broker.rs:6-13`), a
   different type from aien-protocols' `ActionRequest`. No `impl`, no caller.
5. **Still no production constructor.** `aien-mcp/src/effect.rs:40-47` `pub struct
   AuthorizedEffect<T>` with six **private** fields (intent, world_id, winning_jnode,
   policy_digest, capability_digest, idempotency_key); read-only getters `:50-72`; sole
   constructor `authorize_for_test` `:76-93`, `#[cfg(test)] pub(crate)`. Docstring `:35-38` still
   says "no production constructor. The next policy change is the only path that may mint one".
   `EffectIntent` (`:6-11`): `pub` struct, **pub fields** `{provider: ProviderId, tool_name:
   String, arguments: Value, capability_digest: Digest32}`, so anyone can build an intent (fine;
   it carries no authority). `broker.rs:181` `SpeculativeLane::stage_effect_intent(&self, &ProviderId,
   &str, Value) -> Result<EffectIntent, Error>` (refuses speculation-safe tools, `:197`).
   `broker.rs:227` `EffectLane::execute_effect(&self, effect: AuthorizedEffect<EffectIntent>) ->
   Result<EffectReceipt, Error>` (idempotency ledger, stale-digest check `:249-253`).
   `broker.rs:53` `McpBroker::admit(&self, ProviderId, Arc<dyn McpWire>) -> Result<(), Error>`
   (calls `list_tools`, builds session at epoch 1). `SpeculativeLane` `:103`, `EffectLane` `:218`.
   Unchanged since 0.1 (diff empty).
6. **No.** `git grep` for `aien-mcp|aien_mcp|aien-capability|aien_capability` in `*.toml`/`*.rs`
   at origin/main: aien-sovereign-core hits only the workspace member list (`Cargo.toml:5,7`) and
   the two crates themselves; aegis-runtime and aien-protocols: zero. No crate enrolls a
   catalog or depends on either. The only external consumer is INTERPLANE's adapter.
7. **Yes.** aegis-runtime `Cargo.toml:33-38` six path deps `../aien-protocols/crates/*`
   (protocol-types, action-protocol, probe, agent-state-abi, inference-protocol,
   inference-client). Still not consumable as a git dependency.
8. **Approval types** (all inert): aegis-runtime `src/defense/approval.rs:6-11`
   `ApprovalStatus{Pending,Approved,Rejected,Expired}`, `:36` `Approval{id, action_id, run_id,
   session_id, prompt, status, requested_at, decided_at, decided_by}`; SQLite table `approvals`
   `src/persistence/sqlite/approvals.rs:27,52,92,113`; `DoctrineDecision::RequireApproval`
   `src/defense/decision.rs:19`. aien-protocols `crates/aien-action-protocol/src/lib.rs:5`
   `CapabilityGrant{capability, scope, granted_at, expires_at}`, `:42` `ActionAuthorization{
   authorization_id, action_id, approved_by, granted_capabilities, timestamp}`, `:32`
   `ActionRequest`, `:51` `ActionReceipt`. Nothing in any live path creates an `Approval` or
   `ActionAuthorization`. aien-cli `SafetyDecision::AskUser` (`safety.rs:8-12`) is the only
   approval that actually happens (terminal prompt).
9. **Yes, compiles and passes.** In `adapters/aien` with the existing layout
   (`~/workspace/interplane-audit/aegis-runtime` @ f4e8709, `aien-protocols` @ 3a4cdbe, which equal
   origin/main): `cargo build --tests` finished clean. Then with the sovereign-core rev temporarily
   changed to 0bcd558 (edit reverted, Cargo.toml/Cargo.lock restored): `cargo build --tests`
   clean and `cargo test` 10 passed, 0 failed. No checkout was moved. Bump of the rev pin is
   therefore safe but not required (the crates did not change).

## Extra finding: a fourth copy of the guard

`aien-sovereign-core/crates/spark-mail-rs/src/effect.rs:114-116,:263,:449,:461` carries an
in-repo `ProbePolicyGuard` ("Same questions as aegis-runtime `ProbePolicyGuard`", same floor as
`pre_dispatch_check`) and checks it before `mail.send`. This is the only irreversible effect that
asks a policy question today, and it does not use `aien-mcp`. architecture
`docs/02-implementation-status.md` says the same: "There is still no single broker that every
irreversible effect must pass through". Policy mechanisms in tree: A (`pre_dispatch_check`),
B (`SafetyEngine`), mail's copy, and the unproduced `DoctrineDecision`/`ExecutionAuthority`.

## Proposed smallest AIEN-owned seam (not implemented; no fourth policy system)

1. Host: a new `authority.rs` in **aien-mcp** (it owns `AuthorizedEffect`, so the minting code can
   stay crate-private; aien-mcp cannot depend on aegis-runtime because of finding 7).
2. Move (do not copy) the 5-variant `DoctrineDecision` and the `ExecutionAuthority`-shaped trait
   over `EffectIntent` into aien-capability/aien-mcp and have aegis-runtime re-export them; this is
   a relocation of existing types, not a new policy language (feasibility UNVERIFIED, med).
3. Mint is a crate-private `fn mint(intent, verdict_proof)`; the only public entry is
   `EffectAuthority::authorize(&self, EffectIntent, WorldId, JNodeId) -> Result<AuthorizedEffect,
   Refusal>` which calls an injected authority and mints only on `Allow`/`AllowRestricted`.
4. Mapping: Allow and AllowRestricted -> mint (restrictions recorded in `policy_digest`);
   RequireApproval -> `Err(Pending)` that produces a persisted `Approval`, no mint until approved;
   Deny -> `Err(Denied)`; Contain -> `Err(Contained)`, no mint. Fail closed on authority `Err`.
5. Negative control: a `trybuild` compile-fail test (struct literal and `EffectLane::execute_effect(
   EffectIntent)` both rejected, field privacy) plus a unit test that `stage_effect_intent` output
   passed to the authority with a Deny/RequireApproval/Contain stub yields no `AuthorizedEffect`
   and the lane's ledger stays empty. Caveat: ADR 0016 says the real mint authority is the native
   AIENOS root, so the Rust authority must be a thin adapter that asks that root and never
   decides policy itself.
