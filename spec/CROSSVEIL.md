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
   a model puts in `arguments` or `extensions` is never read as approval. The later decision
   arrives through a host-only continuation call (see below), never through the model's channel.
5. Adapter exceptions in `decide` become `denied` with `runtime_unavailable`; in `execute` they become
   `error` with `execution_error`. Fail closed.
6. The adapter, not INTERPLANE, decides the trust class of the returned data
   (`result.provenance.trust` and `result.provenance.content_kind`). Defaults when the adapter says
   nothing: `tool_result` + `unknown` (fail closed; see Crossveil Trust below).

## Approval continuation (host-controlled)

`requires_approval` is not the end of the road, but only the host can continue it. The pipeline
exposes `continue_approval` and `cancel_approval` to the host process; nothing a model produced
(`tool_request`, `arguments`, `extensions`, an envelope, free text) reaches them, and none of it is
read as a grant. The normative contract is in `CORE.md` (Approval continuation); the rules are:

1. **Correlation.** `trace_id` + `request_id` + the `approval_id` the runtime minted for that request
   + the digest of the original `capability_request`. A continuation for changed arguments, another
   capability, another request or an `approval_id` minted for something else is refused: the request
   is DENIED and stays DENIED. The approval covers the exact effect that will run, not a class of
   effects that sounds like it.
2. **Expiry.** `approval.expires_at` as minted is checked against the host's clock. Expired is DENIED.
3. **Catalog.** A runtime catalog that changed between pending and continuation is `stale_capability`
   and DENIED.
4. **Cancellation** by the host is DENIED and terminal.
5. **Replay.** A second continuation of the same request, or of one that is no longer pending, is
   refused and changes nothing. Continuation is a transition on the existing request, never a
   re-admission, so `replayed_message` and `duplicate_request_id` stay on. A runtime's own
   idempotency (a receipt ledger) is separate and is not used for this.
6. **Restart.** Pending state is in memory. After a restart a pending request cannot be continued
   (fail closed); the host starts a new request.
7. `decide` is not called again at continuation, the pipeline still never builds an `authorized`
   decision, and `execute` runs at most once per request.

Evidence: cases A01 to A14 of `bench/PROTOCOL-0.3.md` (`conformance/fixtures/approval/`), identical
verdicts in Rust and Python.

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

5. **InputRecord** (`input.schema.json`) names one piece of material placed in front of the model:
   `input_id`, `content_kind`, `trust`, `source` (a `Party`: who handed the bytes to the context),
   `origin` (a locator: file path, URL, memory record id, provider id or `runtime:<id>`, or a
   digest-only form for sensitive locators), `content_digest`, `trace_id`, `parent_id` (the
   `message_id` or `request_id` that produced it; `null` for host-registered inputs) and
   `derived_from` (`input_id`s it was computed from; may be empty). The runtime assigns `trust`.
   Content the model itself generated is `content_kind = model_generated` with trust
   `external_untrusted`; the pipeline never records it on its own (rule 7). The nine source classes are workspace, web, memory, document, skill
   (including tool descriptions), tool output, external provider, runtime-generated and user
   request; `conformance/fixtures/input/` holds one schema-valid example of each.
6. **Exposure** is the optional `provenance.exposure` on a `tool_request`:
   `{inputs: [input_id...], floor: TrustLevel}`, where `floor` is the least trusted level among the
   inputs visible to the model when it produced the turn. Order: `trusted_runtime` >
   `user_supplied` > `workspace_untrusted` > `external_untrusted`; `unknown` counts as
   `external_untrusted`. The pipeline computes it from its own ledger; an `exposure` arriving inside
   a model-produced or admitted envelope is discarded and recomputed, never read. Since 0.3 cut P3
   the pipeline computes it. Per trace it keeps a ledger of inputs: every result it renders back
   to the model is recorded automatically (`parent_id` is the `request_id`, `trust` and
   `content_kind` are the result's normalized provenance, `unknown` when it has none), and the
   host adds any other input before a turn through a host-only registration call that model
   output and envelopes cannot reach. All `tool_request`s of a turn, and the `CallContext` passed
   to `decide` and `execute`, carry the ledger as it stood when the turn began. Exposure fails
   closed: an empty ledger, an input of unknown or unrecognized trust, and an input whose
   `derived_from` names an id the ledger does not hold all count as `external_untrusted`.
7. **Source classes.** The trust each class receives, and who records it. `Recorded by` is the
   pipeline when it renders a result, otherwise the host through `register_input`. A class that
   can originate outside the workspace is `external_untrusted`; where the runtime cannot tell, it
   labels the weaker value (never the stronger).

   | Source class | `content_kind` | `trust` | Recorded by | Mock source |
   |---|---|---|---|---|
   | workspace | workspace_content | workspace_untrusted | host | file input (case 31) |
   | web | web_content | external_untrusted | pipeline | `web_fetch` (32) |
   | memory | memory | workspace_untrusted | pipeline | `recall_memory` (33) |
   | document | document | external_untrusted | pipeline | `read_document` (34) |
   | skill, tool description | skill | external_untrusted | host (descriptions), pipeline (`load_skill`) | `load_skill`, description input (35) |
   | tool output | tool_result | adapter's value, `unknown` if absent | pipeline | `list_dir` (36) |
   | external provider | external_provider | external_untrusted | pipeline | `call_provider` (37) |
   | runtime-generated | runtime_instruction | trusted_runtime | host | system prompt (38) |
   | user request | user_request | user_supplied | host | operator input (39) |
   | model-generated | model_generated | external_untrusted | host | earlier model text (40) |

   Skills and tool descriptions are `external_untrusted`, not `workspace_untrusted` (P2's example
   said otherwise): a third-party skill or a tool server's description is authored outside the
   workspace and is read by the model as instruction-shaped text. A host that knows a skill file
   was written inside the workspace may register it as `workspace_untrusted`; the mock cannot tell,
   so it labels `external_untrusted`. Memory stays `workspace_untrusted` because the runtime wrote
   it; a memory entry distilled from external content names that content in `derived_from`.
   The pipeline does not record the model's own prior text. When a host feeds earlier model output
   back into the context it registers it as `model_generated` / `external_untrusted` (`parent_id`
   null, `derived_from` naming what the model had seen if known). Recording it automatically would
   make the floor of every multi-turn trace `external_untrusted` by construction and hide real
   exposure changes; an unregistered model turn is a host omission, visible as a ledger that lacks
   it (case 40).

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
