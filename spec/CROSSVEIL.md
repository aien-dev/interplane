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
   (`result.provenance.trust`, `origin`). Default when the adapter says nothing: `untrusted_tool_content` (fail closed).

## State record

Each request's lifecycle (see `CORE.md`) is recorded by the pipeline and exposed to the caller as
an `ObservedRecord`. It is a log, not a lock: the runtime remains free to keep its own ledger.

## Crossveil Trust

Content crossing the boundary carries a `TrustClass`:

| Class | Who authored it | Example |
|---|---|---|
| `runtime_instruction` | the runtime or operator | system prompt, selected tool catalog, approval card |
| `user_request` | the human | the task text |
| `model_proposal` | the model | tool_request, final answer |
| `trusted_runtime_result` | the runtime, from a capability it executed on trusted state | file content from the workspace |
| `untrusted_tool_content` `untrusted_web_content` `untrusted_document` `untrusted_memory` | something outside the runtime's control | fetched web page, inbound email, document, memory written by earlier untrusted content |
| `authorization_decision` | the runtime's policy engine | a decision payload |

Only the runtime assigns a class. A model cannot promote content. Adapters mark results from
origins `web`, `email`, `document`, `memory`, `mcp` as the matching `untrusted_*` class unless the runtime says
otherwise. (Odysseus already does this with `metadata.trusted`; AIEN's aegis-runtime does not yet.)
Conformance includes a fixture where a tool result containing tool-call markup is rendered back:
the markup must come back as data, never be parsed as a new intent.

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
