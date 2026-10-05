# Crossveil: the authority boundary contract

> Nothing crosses the veil as authority merely because a model requested it.

Crossveil is a contract, not a policy engine. A runtime adapter implements two callbacks:

```
decide(capability_request, context) -> decision
execute(capability_request, decision, context) -> result     // called only when decision == authorized
```

`context` carries `trace_id`, `message_id`, `parent_id`, the model `Party`, and any runtime-local
session handle the adapter chose to attach. INTERPLANE passes it through untouched.

Rules the pipeline enforces (and conformance tests):

1. `execute` is called only after `decide` returned `authorized` for that exact `capability_request`.
2. The pipeline never fabricates, caches, upgrades or reuses a decision. Every request is decided.
3. A decision with an unknown `decision` value is treated as `denied` (`unknown_decision`).
4. `requires_approval` returns to the model as `requires_approval`; only a later runtime decision
   that carries `approval.approval_id` minted by the runtime can turn it into `authorized`. Anything
   a model puts in `arguments` or `extensions` is never read as approval.
5. Adapter exceptions in `decide` become `denied` with `runtime_unavailable`; in `execute` they become
   `error` with `execution_error`. Fail closed.
6. The adapter, not INTERPLANE, decides the trust class of the returned data
   (`result.provenance.trust` and `result.provenance.content_kind`). Defaults when the adapter says
   nothing: `tool_result` + `unknown` (fail closed; see Crossveil Trust below).

## State record

Each request's lifecycle (see `CORE.md`) is recorded by the pipeline and exposed to the caller as
an `ObservedRecord`. It is a log, not a lock: the runtime remains free to keep its own ledger.

## Crossveil Trust

Model-visible material carries two axes of provenance, both assigned by the runtime, never by the
model:

| `content_kind` (what it is) | `trust` (how far the runtime trusts it) |
|---|---|
| runtime_instruction, user_request, model_generated, tool_result, workspace_content, memory, web_content, document, email, skill, external_provider, unknown | trusted_runtime, user_supplied, workspace_untrusted, external_untrusted, unknown |

Trust metadata does not block anything by itself; it tells the runtime and the model adapter what
the material is so that a system instruction, a web page, a tool output, a memory hit and a model
proposal are never flattened into indistinguishable text. Rules:

1. Absent or unparseable trust is `unknown`, which receivers treat like `external_untrusted`.
   Inability to determine trust is never converted into trust.
2. Adapters set `result.provenance.content_kind` and `trust` on every result. Defaults when the
   runtime says nothing (absent or null): `tool_result` + `unknown`. A `content_kind` string the
   receiver does not recognize, or a non-string value, is `unknown`. A `trust` string it does not
   recognize, or a non-string value, is `external_untrusted`. `trusted` is always derived from
   `trust` (CORE.md) and an adapter-supplied `trusted` is never read, so unknown provenance never
   becomes trusted by being serialized and read back (conformance cases 26 and 27).
3. Results are rendered back to the model as data. The pipeline never parses tool-call markup inside
   a result into a new intent (conformance cases 22 and 23 carry such markup as an injection probe).
4. Odysseus's `metadata.trusted`/`source` and its `untrusted_context_message` map onto these axes.
   Its `ResultIntegrity` maps explicitly: `workspace_untrusted` and `external_untrusted` carry over;
   `system` (server-authored output) is `trusted_runtime` unless Odysseus's own
   `tool_result_should_arm_gate` marks the result as carrying non-system content, then
   `external_untrusted`. AIEN's aegis-runtime has no equivalent today and the adapter fills in
   `unknown` honestly.

Stage names used by this spec and their decision detail: INVALID is `REJECTED` with decision
`invalid`; NOT_FOUND is `REJECTED` with decision `not_found`. A `capability_request` whose
`mapping.catalog_digest` differs from the runtime's live catalog digest is rejected with
`stale_capability` before the authority callback is invoked.

## Mapping to the two real runtimes

| INTERPLANE decision | Odysseus (2992bf6) | AIEN |
|---|---|---|
| `authorized` | `ToolGateDecision(allowed=True)` and policy not blocking | aien-cli `SafetyDecision::Allow`; aegis `pre_dispatch_check` Ok |
| `denied` | policy `blocks()` / `NON_ADMIN_BLOCKED_TOOLS` / delegated-credential refusal | `SafetyDecision::Deny(reason)`; aegis `Err(reason)`; `DoctrineDecision::Deny` |
| `requires_approval` | `ToolGateDecision(allowed=False)` + sealed `PendingToolApproval` | `SafetyDecision::AskUser(prompt)`; `DoctrineDecision::RequireApproval` |
| `constraints` on authorized | n/a | `DoctrineDecision::AllowRestricted(list)` |
| `not_found` | name not in `TOOL_TAGS` / `_MCP_TOOL_MAP` | skill not in `ALLOWED_SKILLS` / registry |
| `invalid` | `function_call_to_tool_block` rejection | aegis "args must be a JSON object" |

`runtime_state` carries the native detail (`odysseus.tool_gate`, `aien.safety_decision`,
`aien.doctrine_decision`) so nothing is lost in translation.
