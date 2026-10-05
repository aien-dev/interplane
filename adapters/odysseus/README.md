# interplane-adapter-odysseus (reference adapter, AGPL-3.0-or-later)

An INTERPLANE runtime adapter for [Odysseus](https://github.com/odysseus-dev/odysseus), pinned to
commit `2992bf6`. **This adapter never changes Odysseus, and Odysseus keeps all authority.** Every
decision comes from Odysseus's own code; the adapter only translates it into INTERPLANE's
`authorized / denied / requires_approval / not_found / invalid`. If Odysseus cannot be imported,
`decide()` returns `denied` ("odysseus runtime not available"): fail closed, never authorized.
It is licensed AGPL-3.0-or-later (`LICENSE`) because it imports Odysseus (ADR 0002). Core stays Apache-2.0
and needs no change (the `odysseus_text` dialect is registered adapter-locally).

## Files

| File | Job |
|---|---|
| `catalog.odysseus-2992bf6.json` | The record: 71 tools extracted from a live import, with `odysseus.tool_effect` values. Never edited. |
| `interplane_adapter_odysseus/catalog.py` | Loads the record. |
| `.../domains.py` | Hand-curated `TOOL_DOMAINS` (one reviewable line per tool, fixed 19-domain set), 16 `CANONICAL` aliases, `catalog_with_domains()`. |
| `.../mapping.py` | `mapping_table()`: 16 alias rules + 71 passthrough rules, `catalog_digest` from the record. |
| `.../authority.py` | `OdysseusAuthority(RuntimeAuthority)`: `decide`, `execute`, `catalog`. |
| `.../dialect.py` | `odysseus_text` Lenshift dialect and `make_registry()`. |
| `.../demo.py` | Runs turns through the real `Pipeline`. |
| `.../_odysseus.py` | Optional loader for a checkout named by `ODYSSEUS_SRC`. |

## Reused from Odysseus (called for real)

| Odysseus function | Used for |
|---|---|
| `src.agent_tools.TOOL_TAGS` (`agent_tools/__init__.py:79`) | name known? else `not_found` |
| `src.tool_schemas.function_call_to_tool_block` (`tool_schemas.py:1370`; rejects at `:1398`, `:1411`) | argument validity; it only logs the reason, so the adapter captures that log line as the `invalid` reason |
| `ToolRunSecurityContext(...).decision_for` (`tool_capabilities.py:569`, `:654-684`) | the untrusted-context gate; `allowed=False` becomes `requires_approval` |
| `ToolRunSecurityContext.observe_tool_result`, `tool_result_should_arm_gate` (`:506`), `capabilities_for_action` | arming the gate from real results; effects; result integrity (`workspace_untrusted` for file reads) |
| `src.tool_security.is_public_blocked_tool` / `NON_ADMIN_BLOCKED_TOOLS` (`tool_security.py:222`, `:42`), `tool_execution._ADMIN_TOOLS` (`:539`) | admin gates, applied in the order of `execute_tool_block` (gate `:915-926`, then `:1064-1080`) |
| `tool_execution.vet_workspace` (`:466`), `_resolve_tool_path_in_workspace` (`:406`), `_active_workspace` (`:456`) | workspace root check, path confinement (also run in `decide`, so an escape is refused before execution), binding the workspace while a tool runs |
| `agent_tools.TOOL_HANDLERS` -> `ReadFileTool`, `LsTool`, `GlobTool`, `GrepTool` (`filesystem_tools.py:243`, `:613`, ...) | the actual execution of the four read-only tools |
| `tool_execution.format_tool_result` (`:1359`), `prompt_security.untrusted_context_message` (`:64`) | pin the text rendering (differential tests); `round_message()` calls the latter |

## Reimplemented, and why

* **Admin check.** Odysseus's `owner_is_admin_or_single_user` (`tool_security.py:237`) needs its auth
  state (`AuthManager`, config). The adapter takes an `admin` constructor flag instead (default `False`,
  fail closed) and applies Odysseus's own blocklists to it.
* **Per-trace gate state.** The reference pipeline's call context carries only `trace_id`, `message_id`,
  `parent_id`, `model`, `runtime_session`: no prior-result trust. The adapter keeps one real
  `ToolRunSecurityContext` per `trace_id` and arms it with Odysseus's own `observe_tool_result`. A caller
  that tracks results may also pass `trust` / `prior_trust` in the context (`external_untrusted` or
  `workspace_untrusted` arms the gate), or set `external_context_seen=True` on the constructor.
  Odysseus itself treats a workspace read as arming the gate, so a later `send_email` in the same trace
  is `requires_approval`.
* **Result trust.** Odysseus's `ResultIntegrity` (`tool_capabilities.py:37-46`) has a `system` value that
  is not an INTERPLANE `TrustLevel`; the pipeline used to fold it to `external_untrusted` silently. The
  adapter now maps it explicitly: `system` (Odysseus's label for server-authored output) is
  `trusted_runtime` unless Odysseus's own `tool_result_should_arm_gate` (`:506-528`) says the result
  carries non-system content (the producer set `untrusted_content`), then `external_untrusted`.
  `workspace_untrusted` and `external_untrusted` carry over; anything else is `external_untrusted`.
  Limit: `system` is Odysseus's *default* for a registered tool, so a tool Odysseus registers without an
  explicit integrity is labelled `trusted_runtime` through this adapter. None of the four tools the
  adapter executes is `system` at 2992bf6 (all are `workspace_untrusted`), so no executed result
  changes label today.
* **Approval continuation: unsupported on Odysseus (0.3 cut A4, fail closed).** The adapter mints no
  approval id, so a `requires_approval` request has no pending entry in the pipeline and every host
  continuation or cancel is refused (`no_pending_approval`); nothing executes through an approval.
  The decision's `runtime_state.values` carries `approval continuation unsupported on Odysseus`.
  Why not Odysseus's own approvals: `tool_approvals.py` (2992bf6) seals an exact action, but the
  selected scope ("Allow for this task" or "for this chat session") then bypasses the whole
  untrusted-context gate for later actions, and the seal is bound to owner, session, run, selected
  tools and a continuation query that INTERPLANE's single-effect continuation (trace, request,
  minted id, request digest) cannot express. Mapping that onto "this exact effect only" is not
  something this cut can prove, so it refuses instead. `execute` also refuses any decision that is
  not `authorized`. Evidence: `tests/test_approval_continuation.py` (Odysseus subset of
  `bench/PROTOCOL-0.3.md` A01 to A14: A01 unsupported and refused; A02 to A07, A09, A14 refused;
  A08 has nothing pending to go stale; A10 to A13 hold), run with forged continuation decisions on
  purpose.
* **Text rendering.** `render_result` reimplements only the `output` and `error` branches of
  `format_tool_result` (a differential test pins it); Odysseus wraps a whole round once in
  `untrusted_context_message`, available as `dialect.round_message()`.
* **Text parsing.** `odysseus_text` is a forward-only reimplementation of three forms from
  `tool_parsing.py:1288-1346` (fenced, `[TOOL_CALL]`, XML `<tool_call><invoke>`), with the same precedence.
  It never renames or lowercases tools, so Odysseus's `tool => "shell"` stays `shell` and the table answers
  `unknown_capability`; parameter values stay strings (CrossAxis coerces by schema); an unterminated
  call is rejected as truncated, where Odysseus may guess. Hermes JSON `<tool_call>{...}</tool_call>`
  is covered by the core `qwen35` dialect (`hermes_json`) and is not duplicated: here it is rejected with a pointer.
  Odysseus has no literal ```` ```tool ```` tag; its fence tag is the tool name (```` ```bash ````).

## Not executed

In 0.1 only `read_file`, `ls`, `glob`, `grep` execute, inside the workspace given to the constructor.
Every other authorized tool returns `status: error`, `execution_error`, "not executed by the reference
adapter". `execute_tool_block` itself is deliberately not called: its admin check and MCP routing need
Odysseus app state. Not modeled: `tool_policy`/disabled tools, exact-approval replay, scoped approvals.

## Finding

`tail_serve_output` is in the catalog (`tool_schemas.py:899`, executor `tool_execution.py:1205`) but not
in `TOOL_TAGS`, so Odysseus's own native-call path rejects it and the adapter answers `not_found`.
A test pins this.

The recorded `catalog_digest` (`sha256:4ec0d10e...`) is `Catalog.computed_digest()` over the 71
recorded capabilities, so anyone can recompute it from the record with Core alone. (The first
extraction used an ad-hoc formula; it was recomputed on 2026-10-04 with the tools unchanged.)

## Run

```
cd adapters/odysseus
PYTHONDONTWRITEBYTECODE=1 ODYSSEUS_SRC=/path/to/odysseus PYTHONPATH=../../python:. \
  /path/to/odysseus-venv/bin/python -m pytest tests -q -p no:cacheprovider
```
Without `ODYSSEUS_SRC` the Odysseus tests skip with a reason; the catalog, dialect and fail-closed tests still run.
Importing Odysseus would create `data/app.db` in the checkout, so the loader points `ODYSSEUS_DATA_DIR`
at a temp directory (an operator-set value is respected). Demo: `python -m interplane_adapter_odysseus.demo`.
Install: `pip install -e ../../python -e .` (the catalog record sits beside the package, so install in-tree).
