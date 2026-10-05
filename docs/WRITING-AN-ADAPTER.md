# Writing an adapter

An adapter connects INTERPLANE to a host: the program that owns the tools, the policy and the data.
INTERPLANE parses the model's tool calls, maps names, records what the model has seen (exposure) and
enforces approvals. It never decides. Every decision is the host's.

Working example: `examples/adapter/todo_adapter.py` (a to-do list host, about 150 lines, self-checking).
Run it with `python3 examples/adapter/todo_adapter.py`. This guide walks through it.

The Python names used here are all public: `interplane.lenshift`, `interplane.crossaxis.MappingTable`
and `Rule`, and from `interplane.crossveil`: `Pipeline`, `Catalog`, `CapabilityDescriptor`, `ToolRef`,
`Decision`, `make_result`, `ContinuationRefused`.

## 1. The authority: three methods

The host object needs a `runtime_id` and three methods (`RuntimeAuthority` in `crossveil`):

| Method | Called | Returns |
|---|---|---|
| `catalog()` | before `decide`, to check the mapping's catalog digest and look up the capability (also on continuation) | a `Catalog` of `CapabilityDescriptor`s |
| `decide(req, ctx)` | once per admitted request, before anything runs | a `Decision` |
| `execute(req, decision, ctx)` | only after `authorized` (or an approved continuation) | a `ToolResult` from `make_result` |

`req` is a `CapabilityRequest` (`request_id`, `capability`, `arguments`). `ctx` is a plain `dict` with
`trace_id`, `message_id`, `parent_id`, `model`, `runtime_session` (`None` unless the host attaches one)
and `exposure`, where `exposure` is
`{"floor": <trust level>, "inputs": [<input_id>...]}`. INTERPLANE passes it through untouched
(CROSSVEIL.md, the authority boundary contract).

A `decide` or `execute` that raises does not crash the turn. A raising `decide` is a denial
(`runtime_unavailable`), a raising `execute` a failed request (`execution_error`). See CORE.md,
"Adapter and catalog integrity".

## 2. Decisions

`Decision(request_id, decision, authority=..., capability=..., reason=..., approval=...)`. The values
are `authorized`, `denied`, `requires_approval`, `not_found` and `invalid`. For `requires_approval`,
set `approval={"approval_id": ..., "scope": "single_action", "expires_at": ...}`. The host mints the
id. The pipeline holds the request and returns a refusal to the model.

Policy is entirely the host's. The example authorizes reads, always holds deletes, and holds adds
when the exposure floor is below `user_supplied` (anything untrusted is in view). INTERPLANE does not
ship a policy engine. The mock runtime's policy (CORE.md, "Mock runtime") is one example of the
pattern.

## 3. Results and labels

`make_result(request_id, "ok", runtime=..., capability=..., data=..., content_kind=..., trust=...)`
builds a result with the canonical shape. The host labels every executed result. The label decides
what later turns are exposed to:

- `trust` is one of `trusted_runtime`, `user_supplied`, `workspace_untrusted`, `external_untrusted`.
  The ordering is the reverse, from least to most trusted: `external_untrusted` <
  `workspace_untrusted` < `user_supplied` < `trusted_runtime`.
- Label what the bytes could contain, not where they came from. In the example, the to-do list is
  `workspace_untrusted`, because item text can come from anywhere. The host's own "added item-1" reply
  is `trusted_runtime`, because the host wrote it.
- An absent or unrecognized label fails closed (CROSSVEIL.md, Crossveil Trust rules 1 and 2).

## 4. Exposure, and what the host registers

The pipeline records every result it renders back to the model. Inputs it never sees, such as the
user's message, a file the host pasted in, or a system prompt, the host registers before the turn
with `pipe.register_input({...})`, an InputRecord (`spec/schemas/input.schema.json`). With an empty
ledger the floor is `external_untrusted`, so register the user's request first if your policy
trusts it.

One behaviour to know about: when a request is refused or held, the pipeline's reply to the model
carries no labels, and it is recorded as `unknown`, which counts as `external_untrusted`. After a
single refusal, the floor for the rest of the trace is `external_untrusted`. The example shows this at
step 5. It is the spec as written and fails safe; whether it should change is
[issue #57](https://github.com/aien-dev/interplane/issues/57).

## 5. Wiring the pipeline

```python
pipe = Pipeline(registry=lenshift, mapping_table=mapping_for(host.catalog()), runtime=host)
outcome = pipe.run_turn("openai", "my-model", assistant_message, trace_id, turn_number)
```

The mapping table names how model-facing tool names reach catalog capabilities. A one-to-one
`Rule("passthrough:<name>", "passthrough", None, name, name)` per capability is enough to start. Bind
the table to the catalog with `catalog_digest=catalog.computed_digest()`, so a catalog change is
detected as stale. `outcome.observed` holds one `ObservedRecord` per call, with `request_id`, `stage`,
`decision`, `decide_invoked`, `execute_invoked` and more.

Dialects: `lenshift.names()` lists them (`openai`, `openai_stream`, `qwen35`, `aien_legacy`). `openai`
takes the assistant message with `tool_calls`; `qwen35` takes Qwen 3.5's XML tool-call text.

## 6. Approvals

When a person approves a held request, the host continues it:

```python
pending = pipe.pending_approval(trace_id, request_id)          # host-only view
answer = Decision(request_id, "authorized", authority=..., approval={"approval_id": pending.approval_id})
result, record = pipe.continue_approval(trace_id, request_id, answer, pending.request_digest, now)
```

The pipeline checks the request id, the approval id, the request digest, expiry and the catalog
digest, in the order pinned in CORE.md ("Approval continuation"). It executes at most once. A
`denied` answer, or any failed check, ends the request for good. A second continuation raises
`ContinuationRefused`. `continue_approval` never calls `decide` again.

## 7. Checking an adapter against the conformance corpus

`spec/CORE.md`, "Adapter subsets" and its "Adding an adapter" paragraph, say which fixtures an
adapter must run and how they are judged. `conformance/runners/adapter_runner.py` runs them and
judges them, so an adapter author writes two files and no runner:

- `authority.py` defines `RUNTIME`, `POLICY`, `make_authority(*, no_exposure_check, expires_at)` and
  `make_pipeline(authority)`. The authority counts its calls in `decide_calls` and `execute_calls`,
  mints approval ids with the given `expires_at`, and when `no_exposure_check` is true drops its
  exposure floor check (the negative control).
- `entry.json` is `{"<name>": <table entry>}`, with the fields of the `aien` entry in
  `conformance/adapter-translation.json` and the effect classes of "Adding an adapter".

```sh
python3 conformance/runners/adapter_runner.py --module authority.py --entry entry.json --out verdicts.json
python3 conformance/runners/adapter_runner.py --module authority.py --entry entry.json --no-exposure-check
```

The first should exit 0 with `violations` 0 and `content_derived` 0. The second should exit 1 with
the injection cases failing; if it passes, the policy is not what stops the injections. The entry is
added to an in-memory copy of the table, so the shared table and `TRUST-DIGEST.txt` stay unchanged
and the subset is computed, not frozen (say so in any report). Approval cases are judged by the
A-AIEN rule, so the runner expects approval continuation. `adapter_subset.py --plans <name>` prints
the translated plans of an adapter already in the table.

Two runs of this process are recorded: an independent one (`docs/REPORT-adapter-repro.md`) and a
maintainer check on a smart-home host (its addendum). The shared runner reproduces both byte for
byte (`python/tests/test_adapter_runner.py`).
