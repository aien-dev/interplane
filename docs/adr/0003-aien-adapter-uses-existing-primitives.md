# ADR 0003: The AIEN adapter uses AIEN's existing primitives and mints nothing

Status: Accepted (2026-10-04)

Observed (CURRENT_STATE.md, verified): `aien-capability::ToolDescriptor{name, effects, schema_digest}`
with an order-independent `catalog_digest`; `aien-mcp::McpBroker::admit` records that digest,
`SpeculativeLane::invoke_speculative` refuses tools whose effects are not speculation-safe,
`stage_effect_intent` records an intent without executing, `EffectLane::execute_effect` requires an
`AuthorizedEffect` that has no production constructor. Live authority decisions today are
`aien-cli::SafetyEngine::evaluate -> SafetyDecision{Allow, AskUser, Deny}` and
`aegis::enforcement::pre_dispatch_check -> Result<(), String>`. `DoctrineDecision::RequireApproval`
exists but nothing produces it. No crate depends on aien-mcp or aien-capability.

Decision: `adapters/aien` (Rust crate `interplane-adapter-aien`, AGPL-3.0-or-later) implements
`RuntimeAuthority` by composing, not replacing:

1. **Catalog**: built from `aien-capability` descriptors; `catalog_digest` is AIEN's own
   `catalog_digest()`; CrossAxis `stale_capability` protection reuses it.
2. **Decide**: effects classification from `ToolEffects`; `speculation_safe` tools (reads) are
   decided by `SafetyEngine::evaluate` (Allow -> authorized, AskUser -> requires_approval,
   Deny -> denied) and `pre_dispatch_check` (Err -> denied) in that order; any tool with
   WORLD_MUTATION | EXTERNAL_WRITE | EXTERNAL_IRREVERSIBLE is `requires_approval` at best, because
   AIEN itself cannot mint an `AuthorizedEffect` outside tests, and the adapter will not pretend.
3. **Execute**: reads go through `aien-mcp::SpeculativeLane::invoke_speculative` against a
   `McpWire` (in 0.1 an in-process `LocalToolServer` exposing read_file/list_dir over the workspace);
   effects go through `stage_effect_intent` only, returning `requires_approval` with the staged
   intent id in `runtime_state` so an operator-side AEGIS flow can later authorize it.
4. Nothing in the adapter constructs `AuthorizedEffect`, `DoctrineDecision::Allow`, or a
   `SafetyDecision`. Every decision object the adapter emits is a translation of a value AIEN produced.

Consequence: DoD items "AIEN adapter executes an authorized read" and "refuses an unauthorized
effect" are both satisfied by AIEN's current machinery; "approval flow to execution" is honestly
`requires_approval` until AIEN ships a production `AuthorizedEffect` mint (its own ADR 0016 R8 says
only the AIENOS root does that). The adapter depends on `aien-sovereign-core` crates by git
revision 7580039 and never the reverse.
