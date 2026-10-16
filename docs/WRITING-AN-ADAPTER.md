# Writing an adapter

An adapter connects INTERPLANE to a host: the program that owns the tools, the policy and the data.
INTERPLANE parses the model's tool calls, maps names, records what the model has seen (exposure) and
enforces approvals. It never decides. Every decision is the host's.

Working example: `rust/crates/interplane-crossveil/examples/todo_adapter.rs` (a to-do list host,
self-checking). Run it from `rust/` with `cargo run -q -p interplane-crossveil --example todo_adapter`.
This guide walks through it.

The Rust names used here are all public: `interplane_lenshift::DialectRegistry`,
`interplane_crossaxis::MappingTable`, from `interplane_crossveil`: `Pipeline`, `RuntimeAuthority`,
`CallContext`, `Refusal`, and from `interplane_core`: `Catalog`, `CapabilityRequest`, `Decision`,
`ToolResult`, `ResultProvenance`, `TrustLevel`, `InputRecord`, `Limits`, `RequestLedger`.

## 1. The authority: three methods

The host implements `RuntimeAuthority` (in `interplane-crossveil`): a `runtime_id()` and three methods.

| Method | Called | Returns |
|---|---|---|
| `catalog()` | before `decide`, to check the mapping's catalog digest and look up the capability (also on continuation) | a `Catalog` of `CapabilityDescriptor`s |
| `decide(req, ctx)` | once per admitted request, before anything runs | a `Decision` |
| `execute(req, decision, ctx)` | only after `authorized` (or an approved continuation) | a `ToolResult` |

`req` is a `CapabilityRequest` (`request_id`, `capability`, `arguments`). `ctx` is a `CallContext` with
`trace_id`, `message_id`, `parent_id`, `model`, `session` (`None` unless the host attaches one) and
`exposure`, an `Option<Exposure>` with `floor` (a trust level) and `inputs` (input ids). INTERPLANE
passes it through untouched (CROSSVEIL.md, the authority boundary contract).

A `decide` or `execute` that panics does not crash the turn. A panicking `decide` is a denial
(`runtime_unavailable`), a panicking `execute` a failed request (`execution_error`). See CORE.md,
"Adapter and catalog integrity".

## 2. Decisions

A `Decision` carries `request_id`, `decision`, `authority` (`runtime`, `policy_engine`, optional
`decision_id`), `capability`, `reason` and `approval`. The values are `authorized`, `denied`,
`requires_approval`, `not_found` and `invalid`. For `requires_approval`, set
`approval` to `{"approval_id": ..., "scope": "single_action", "expires_at": ...}`. The host mints the
id. The pipeline holds the request and returns a refusal to the model. The example builds its
decisions from JSON with `serde_json::from_value`, which keeps them close to the wire form.

Policy is entirely the host's. The example authorizes reads, always holds deletes, and holds adds
when the exposure floor is below `user_supplied` (anything untrusted is in view). INTERPLANE does not
ship a policy engine. The mock runtime's policy (CORE.md, "Mock runtime") is one example of the
pattern.

## 3. Results and labels

`ToolResult::ok(request_id, data)` builds a result with the canonical shape; the host then sets its
`provenance` (`capability`, `runtime`, `content_kind`, `trust`, and `trusted` from
`TrustLevel::trusted_flag`). The host labels every executed result. The label decides what later
turns are exposed to:

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
with `pipe.register_input(record)`, an `InputRecord` (`spec/schemas/input.schema.json`). With an
empty ledger the floor is `external_untrusted`, so register the user's request first if your policy
trusts it.

One behaviour to know about: when a request is refused or held, the pipeline's reply to the model
carries no labels, and it is recorded as `unknown`, which counts as `external_untrusted`. After a
single refusal, the floor for the rest of the trace is `external_untrusted`. The example shows this at
step 5. It is the spec as written and fails safe; whether it should change is
[issue #57](https://github.com/aien-spark/interplane/issues/57).

## 5. Wiring the pipeline

```rust
let mut pipe = Pipeline::new(DialectRegistry::with_defaults(), table, &mut host,
                             Limits::default(), RequestLedger::new());
let outcome = pipe.run_turn("openai", "my-model", &assistant_message, trace_id, turn_number);
```

The mapping table names how model-facing tool names reach catalog capabilities. One `passthrough`
rule per capability (`{"id": "passthrough:<name>", "kind": "passthrough", "from": {"namespace":
null, "name": <name>}, "to": <name>}`) is enough to start. Bind the table to the catalog with
`catalog_digest: catalog.compute_digest()`, so a catalog change is detected as stale.
`outcome.records` holds one `ObservedRecord` per call, with `request_id`, `stage`, `decision`,
`decide_invoked`, `execute_invoked` and more.

Dialects: `DialectRegistry::names()` lists them (`openai`, `openai_stream`, `qwen35`, `aien_legacy`).
`openai` takes the assistant message with `tool_calls`; `qwen35` takes Qwen 3.5's XML tool-call text.

## 6. Approvals

When a person approves a held request, the host continues it:

```rust
let pending = pipe.pending_approval(trace_id, request_id);       // host-only view
// answer: a Decision "authorized" with approval {"approval_id": pending.approval_id}
let (result, record) = pipe.continue_approval(trace_id, request_id, &answer, &pending.request_digest, now)?;
```

The pipeline checks the request id, the approval id, the request digest, expiry and the catalog
digest, in the order pinned in CORE.md ("Approval continuation"). It executes at most once. A
`denied` answer, or any failed check, ends the request for good. A second continuation returns
`Err(Refusal::NoPendingApproval)`. `continue_approval` never calls `decide` again.

## 7. Checking an adapter against the conformance corpus

`spec/CORE.md`, "Adapter subsets" and its "Adding an adapter" paragraph, say which fixtures an
adapter must run and how they are judged. `interplane_conformance::adapter_runner` runs them and
judges them, so an adapter author writes a host and a table entry, and no runner:

- An `AdapterHost`: `runtime()`, `policy()`, `make_authority(no_exposure_check, expires_at)` (a fresh
  authority per case that mints approval ids with the given `expires_at`, and drops its exposure
  floor check when `no_exposure_check` is true: the negative control) and `make_pipeline(authority)`.
- An entry `{"<name>": <table entry>}`, with the fields of the `aien` entry in
  `conformance/adapter-translation.json` and the effect classes of "Adding an adapter".

```rust
let v = adapter_runner::run(repo_root, "<name>", Some(&entry), &host, false)?;  // must pass
let n = adapter_runner::run(repo_root, "<name>", Some(&entry), &host, true)?;   // must fail
```

The first should pass with `violations` 0 and `content_derived` 0. The second should fail with the
injection cases failing; if it passes, the policy is not what stops the injections. The entry is
added to an in-memory copy of the table, so the shared table and `TRUST-DIGEST.txt` stay unchanged
and the subset is computed, not frozen (say so in any report). Approval cases are judged by the
A-AIEN rule, so the runner expects approval continuation. `interplane-corpus subset --plans <name>`
prints the translated plans of an adapter already in the table.

Two runs of this process are recorded: an independent one (`docs/REPORT-adapter-repro.md`) and a
maintainer check on a smart-home host (its addendum). Both hosts are ported to Rust in
`rust/crates/interplane-conformance/tests/hosts/`, and `tests/adapter_runner.rs` reproduces both
recorded runs.
