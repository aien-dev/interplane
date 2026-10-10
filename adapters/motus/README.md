# interplane-adapter-motus

Reference adapter that lets a [Lithos AI Motus](https://pypi.org/project/lithosai-motus/) agent send
its tool calls through INTERPLANE and produce evidence that can be verified later without trusting
the host. License: Apache-2.0 (see `LICENSE`). Motus is Apache-2.0, so unlike the Odysseus and AIEN
adapters (AGPL hosts) there is no copyleft to match; this follows the Core licence in
`docs/adr/0002-licensing.md`. Python is allowed here only because Motus itself is Python; nothing in
this directory belongs on AIEN's production runtime path.

**The adapter never decides.** It builds no `Decision` and no `ToolResult`. Every decision and every
execution comes from a `RuntimeAuthority` that the host supplies. If the authority is missing,
unreachable or confused, the call is refused and nothing runs. Motus is an optional extra, never a
hard dependency. INTERPLANE Core is not modified.

## Upstream pin

* Package: `lithosai-motus==0.4.3` (PyPI, 2026-05-29), sdist sha256
  `b1f50bf3ef750b45c6db96d056c4f44b30a2b90aad073e72860f19bb56b8b096`.
* `github.com/lithos-ai/motus` returned HTTP 404 on 2026-10-10 (removed or made private), so there is
  no upstream commit to pin. The PyPI sdist is the only surviving upstream; every Motus symbol used
  below was read from it.

## The seam, with evidence (all paths inside the 0.4.3 sdist, `src/motus/`)

| Fact | Where |
|---|---|
| `ReActAgent` takes `tools` (a `Tools`, dict, list or callable) | `agent/react_agent.py:41`, docs `:62` |
| The loop calls `self.tools[name](arguments_string)` and awaits the result into the conversation | `agent/react_agent.py:209-221`, `:241` |
| `Tool` is a public base class with `name`, `description`, `json_schema` and `__call__(args: str)` | `tools/core/tool.py:10`, `:20-30`, `:72` |
| `Tool._execute` is the traced task (`@agent_task(task_type=TOOL_CALL)`); `_invoke` is the abstract hook | `tools/core/tool.py:84-85`, `:113-114` |
| A `Tool` instance in `tools=[...]` is accepted as is | `tools/core/normalize.py:285-291` |
| What the model is shown comes from each tool's `json_schema` and `description` | `agent/tasks.py:51-76` |
| Motus's own tracer id is a UUID, not exposed to a tool | `runtime/tracing/agent_tracer.py:88` |

The adapter therefore subclasses the public `Tool` (`motus_tool.interplane_tool_class`). It overrides
`__call__`, keeps the `TOOL_CALL` task so Motus tracing still sees the call, holds no implementation,
and hands the raw argument string to the gate. It does not patch the runtime, does not override
`_dispatch_tool_call` (private) and does not use `requires_approval` (that is Motus's own interrupt,
not an INTERPLANE approval). No `.policy()` retry is set on the task, and the adapter never retries.

Limit: `Tool.__call__` receives only the argument string, so Motus's tool-call id
(`react_agent.py:220`) and task id are not visible to a tool. The evidence labels them unavailable
unless the host passes `motus_ids`.

## How a call flows

```
Motus ReActAgent -> InterplaneTool(args: str) -> ToolBridge -> Gate.call
   -> INTERPLANE Pipeline (Lenshift openai dialect -> Core -> CrossAxis -> decide -> execute)
   -> RuntimeAuthority            (the only place a decision or an execution comes from)
   -> text back to Motus, one evidence record
```

* `trace_id`: 32 lowercase hex characters, minted once per `Gate` (one Motus agent run), carried on every
  envelope and echoed in every record (contract C1 in the six-issue coordination record).
* `request_id` (`mc-000001`, ...) and `message_id` (`motus-<trace>-<n>-0`) are minted per call. The
  Pipeline ledger rejects a repeated `request_id` in a trace; the record shows `duplicate_request_id`.
* Outcomes are preserved: `authorized`, `denied`, `requires_approval`, `not_found`, `invalid`, stale
  catalog, timeout, cancellation, uncertain effect, authority unavailable. Labels in the evidence use the
  contract C2 words: `finished`, `rejected`, `approval_pending`, `timeout`, `cancelled`, `uncertain`,
  `failed`, `unavailable`, with `effect_certainty` of `none`, `uncertain` or `occurred`. After an execute
  request has been written, only an explicit ok result is labelled `occurred`; any error is `uncertain`.
  A bridge already gone before the request was written is `unavailable` with `none`.
* Model-visible text is whatever the Pipeline renders. Text inside a tool result (including text that
  claims authorization) is data; it changes no decision and the next effect is decided afresh.
* Approval continuation is host-only: `Gate.approve(request_id, approver)`. The `approver` is the
  authority's side and returns the runtime's own continuation decision. The gate binds it to the
  request digest recorded when the authority was asked; the Pipeline re-checks id, digest, expiry and
  catalog and executes at most once. A second continuation is refused before the authority is asked.

## Authorities

1. **Core mock runtime** (`interplane.crossveil.MockRuntime`) for unit tests.
2. **`tests/fake_authority.py`** (TEST DOUBLE, never ship): a small read-only authority used only by
   the offline gate tests. It lives under `tests/`, is not part of the package, and the demo does not
   use it. The package mints no authorization: `aien_bridge.py` builds only a `denied` Decision.
3. **`AienBridgeAuthority`** (this package): the **real AIEN authority**. `adapters/aien` (Rust)
   ships the binary `aien-authority-bridge`, which wraps `AienAuthority` over a workspace and speaks
   one JSON line per call on stdin and stdout. See "AIEN-backed authority (real path)" below.

## Install and run

```
python3 -m venv .venv-motus && . .venv-motus/bin/activate
pip install -e python -e adapters/motus pytest          # from the repository root
python -m pytest adapters/motus/tests -q                # offline, no credentials
python -m interplane_adapter_motus.verify out.json      # the demo needs the AIEN bridge, see below
```

Optional, to run against the real Motus (pulls about twenty dependencies; do it in its own venv):

```
pip install -e "adapters/motus[motus]"                  # pins lithosai-motus==0.4.3
python -m pytest adapters/motus/tests -q                # the Motus tests run instead of skipping
```

Without Motus, `interplane_tool_class()` raises `MotusNotInstalled` and the host stays on the tiny
`FakeHostTool` (same shape as a Motus tool); the demo says so. Nothing is authorized either way.
The demo has no stand-in authority: without `--aien` it prints a plain message and exits non-zero.

## AIEN-backed authority (real path)

`AienBridgeAuthority` spawns `aien-authority-bridge` and forwards every `decide` and `execute` to
AIEN's own `AienAuthority` (pinned in `adapters/aien/Cargo.toml`). Answers are the serde JSON of the
real `Catalog`, `Decision` and `ToolResult`, parsed with `interplane.core`; the bridge and the Python
class add no authority and invent no schema. The mapping table also comes from AIEN
(`AienBridgeAuthority.mapping_table()`), and the evidence header records the authority `runtime_id`
reported by the bridge (`aien`). A held request is continued through AIEN's own approval desk (op
`approve`: one single-use grant for exactly that request, spent by AIEN; the bridge allows one
approve attempt per request id, and `execute` only for a request it decided `authorized` itself,
once; a decision supplied by the client is never trusted. Request ids are `<trace_id>-<turn>` so
runs on one bridge never collide); declining is `Gate.cancel_approval`.

Fail closed: if the binary is missing, will not start, answers with malformed JSON or a decision for
another request id, answers with extra bytes after a response, exceeds the timeout (10 s default, writes included) or exits in the middle of a call, `decide`
returns `denied` with reason `aien bridge unavailable`, the bridge is not used again, and `execute`
is never reached. A bridge lost during `execute` is reported as an uncertain effect and never retried.

Build and run (the sibling checkouts in `adapters/aien/PINS` are needed; see `adapters/aien/README.md`):

```
adapters/aien/setup-siblings.sh                                   # from the repository root
(cd adapters/aien && cargo build --bin aien-authority-bridge)
export INTERPLANE_AIEN_BRIDGE=$PWD/adapters/aien/target/debug/aien-authority-bridge
cd adapters/motus
python -m interplane_adapter_motus.demo --aien /path/to/workspace out.json   # prints "Authority: AIEN (adapters/aien via bridge)"
python -m interplane_adapter_motus.verify out.json
```

Tests (offline, no credentials, CPU only; no GB10 claim):

```
(cd adapters/aien && cargo test --test bridge)                    # Rust: the binary driven over a pipe
cd adapters/motus && python -m pytest tests/test_aien_bridge.py -q -rs
```

The fail-closed tests use fake bridge scripts and always run. The end-to-end class (authorized read,
path escape denied with no read, malformed arguments, duplicate request id, held write with a
single-use continuation, evidence verifies, tampered bundle fails) runs against the real binary and
skips with a reason when it is not built. CI job `adapter-motus` builds the bridge and fails if those
tests skip.

## Evidence bundle and verifier

`Gate.evidence()` returns a JSON-able bundle: `schema`, `trace_id`, `header`, `records`, `bundle_digest`.
Each record holds `request_id`, `message_id`, the Motus ids (or null with the header label), digests of the
arguments, the capability request, the decision, the result and the text shown to the model, the
decision, `executed`, `outcome`, `effect_certainty` and `retried` (always false). Every digest is
`sha256:` over RFC 8785 canonical JSON (`interplane.core.digest`). A record digest covers all of its other
fields; the bundle digest covers schema, trace id, header and the ordered record digests.

`python -m interplane_adapter_motus.verify bundle.json` recomputes all of it and also checks: seq is
contiguous, one trace id throughout, nothing `executed` without an `authorized` decision, no request
executed twice, a continuation only for a request that was held, no retries. Exit 0 means untampered
and internally consistent. It does not prove the authority was honest, and tracing is observation, not
proof of authorization; the receipts and digests are the proof. Evidence that does not exist (for
example a capability request for a call refused before the authority) is labelled `unavailable`, never
invented.

## Not done

* No live Motus plus AIEN run: the AIEN path is exercised through the host-neutral gate and the fake host tool; a real Motus agent loop over the bridge is not run. No GB10 path.
* Backends are synchronous behind the authority; wrapping an existing async Motus tool is not provided.
* Approvals live in memory (a restart refuses every continuation, fail closed).
* The real-Motus tests are not in CI (Motus's dependencies are not in `constraints/ci-python.txt`).
