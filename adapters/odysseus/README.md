# interplane-adapter-odysseus (reference adapter, AGPL-3.0-or-later)

An INTERPLANE runtime adapter for [Odysseus](https://github.com/odysseus-dev/odysseus), pinned to
commit `a8c147b` (2026-10-07; re-pinned from `2992bf6` in #95). **This adapter never changes Odysseus, and Odysseus keeps all authority.** Every
decision comes from Odysseus's own code; the adapter only translates it into INTERPLANE's
`authorized / denied / requires_approval / not_found / invalid`. If Odysseus cannot be imported,
`decide()` returns `denied` ("odysseus runtime not available"): fail closed, never authorized.
It is licensed AGPL-3.0-or-later (`LICENSE`) because it imports Odysseus (ADR 0002). Core stays Apache-2.0
and needs no change (the `odysseus_text` dialect is registered adapter-locally).

## Files

| File | Job |
|---|---|
| `catalog.odysseus-a8c147b.json` | The record: 88 tools extracted from a live import, with `odysseus.tool_effect` values. Never edited; regenerate with `tools/extract_catalog.py`. |
| `tools/extract_catalog.py` | The extraction (#95): imports Odysseus, reads `FUNCTION_TOOL_SCHEMAS` and `capabilities_for_action(name, "")`, sorts by name, takes digests from Core. Run against `2992bf6` it reproduces the old record byte for byte (checked when it was written). |
| `interplane_adapter_odysseus/catalog.py` | Loads the record. |
| `.../domains.py` | Hand-curated `TOOL_DOMAINS` (one reviewable line per tool, fixed 19-domain set; the 17 tools new at a8c147b were assigned by hand), 16 `CANONICAL` aliases, `catalog_with_domains()`. |
| `.../mapping.py` | `mapping_table()`: 16 alias rules + 88 passthrough rules, `catalog_digest` from the record. |
| `.../authority.py` | `OdysseusAuthority(RuntimeAuthority)`: `decide`, `execute`, `catalog`. |
| `.../launch.py` | `launch_preflight()`: structural process-launch check (#93). Never authorizes, never executes. |
| `.../dialect.py` | `odysseus_text` Lenshift dialect and `make_registry()`. |
| `.../demo.py` | Runs turns through the real `Pipeline`. |
| `.../_odysseus.py` | Optional loader for a checkout named by `ODYSSEUS_SRC`. |

## Reused from Odysseus (called for real)

| Odysseus function | Used for |
|---|---|
| `src.agent_tools.TOOL_TAGS` (defined `tool_types.py:13`, re-exported by `agent_tools`) | name known? else `not_found` |
| `src.tool_schemas.function_call_to_tool_block` (`tool_schemas.py:2048`; rejects at `:2108`, `:2161`) | argument validity; it only logs the reason, so the adapter captures that log line as the `invalid` reason |
| `ToolRunSecurityContext(...).decision_for` (`tool_capabilities.py:818`, body `:818-876`) | the untrusted-context gate; `allowed=False` becomes `requires_approval` |
| `ToolRunSecurityContext.observe_tool_result`, `tool_result_should_arm_gate` (`:619`), `capabilities_for_action` | arming the gate from real results; effects; result integrity (`workspace_untrusted` for file reads) |
| `src.tool_security.is_public_blocked_tool` / `NON_ADMIN_BLOCKED_TOOLS` (`tool_security.py:227`, `:45`), `tool_execution._ADMIN_TOOLS` (`:1269`) | admin gates, applied in the order of `execute_tool_block` (gate `:1858-1869`, then admin `:2032-2036` and public-blocked `:2060-2074`) |
| `tool_execution.vet_workspace` (`:1179`), `_resolve_tool_path_in_workspace` (`:1096`), `_active_workspace` (`:1149`) | workspace root check, path confinement (also run in `decide`, so an escape is refused before execution), binding the workspace while a tool runs |
| `agent_tools.TOOL_HANDLERS` -> `ReadFileTool`, `LsTool`, `GlobTool`, `GrepTool` (`filesystem_tools.py:278`, `:786`, `:840`, `:947`) | the actual execution of the four read-only tools |
| `tool_execution.format_tool_result` (`:2485`), `prompt_security.untrusted_context_message` (`:64`) | pin the text rendering (differential tests); `round_message()` calls the latter |

## Reimplemented, and why

* **Admin check.** Odysseus's `owner_is_admin_or_single_user` (`tool_security.py:237`) needs its auth
  state (`AuthManager`, config). The adapter takes an `admin` constructor flag instead (default `False`,
  fail closed) and applies Odysseus's own blocklists to it.
* **Per-trace gate state, armed from exposure (0.3 cut E2).** The adapter keeps one real
  `ToolRunSecurityContext` per `trace_id` and arms it two ways: Odysseus's own `observe_tool_result` on
  every result it executes (Odysseus treats a workspace read as arming the gate), and the pipeline's
  `CallContext.exposure`. An exposure floor below `user_supplied` (`workspace_untrusted` or
  `external_untrusted`), or a missing or malformed exposure, arms the gate (fail closed); once armed it
  stays armed for the trace. The pipeline computes exposure from its own input ledger, so content the
  host registered (web, memory, documents, skills, tool descriptions) arms the gate even though Odysseus
  never executed it. The caller-supplied `trust` / `prior_trust` hints of 0.1 are gone. A host registers
  the user's request first (`demo.register_user_turn`); an empty ledger is `external_untrusted`, so
  without it every effect in the trace is `requires_approval`. `external_context_seen=True` on the
  constructor still arms every trace from the start. Evidence: `tests/test_injection_exposure.py`
  (5 host-registered sources and a workspace read, against `send_email`, `write_file`, `web_fetch`:
  all `requires_approval`, 0 executed; the same calls with only the user request are `authorized`).
* **Result trust.** Odysseus's `ResultIntegrity` (`tool_capabilities.py:39-42`) has a `system` value that
  is not an INTERPLANE `TrustLevel`; the pipeline used to fold it to `external_untrusted` silently. The
  adapter now maps it explicitly: `system` (Odysseus's label for server-authored output) is
  `trusted_runtime` unless Odysseus's own `tool_result_should_arm_gate` (`:619-664`) says the result
  carries non-system content (the producer set `untrusted_content`), then `external_untrusted`.
  `workspace_untrusted` and `external_untrusted` carry over; anything else is `external_untrusted`.
  Limit: `system` is Odysseus's *default* for a registered tool, so a tool Odysseus registers without an
  explicit integrity is labelled `trusted_runtime` through this adapter. None of the four tools the
  adapter executes is `system` at a8c147b (all are `workspace_untrusted`), so no executed result
  changes label today.
* **Approval continuation: unsupported on Odysseus (0.3 cut A4, fail closed).** The adapter mints no
  approval id, so a `requires_approval` request has no pending entry in the pipeline and every host
  continuation or cancel is refused (`no_pending_approval`); nothing executes through an approval.
  The decision's `runtime_state.values` carries `approval continuation unsupported on Odysseus`.
  Why not Odysseus's own approvals: `tool_approvals.py` (a8c147b) seals an exact action, but the
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
  `tool_parsing.py:1850-2013` (fenced, `[TOOL_CALL]`, XML `<tool_call><invoke>`), with the same precedence.
  It never renames or lowercases tools, so Odysseus's `tool => "shell"` stays `shell` and the table answers
  `unknown_capability`; parameter values stay strings (CrossAxis coerces by schema); an unterminated
  call is rejected as truncated, where Odysseus may guess. Known drift at a8c147b: upstream now skips
  fenced blocks when a non-fenced call envelope is in the same text (`_contains_explicit_tool_markup`,
  `tool_parsing.py:271`, `:1889`) and still dispatches an empty fence for email tools; this dialect does neither
  (it has no email fence tags). Both sides' calls still cross the same gates; this is parser fidelity, not authority. Hermes JSON `<tool_call>{...}</tool_call>`
  is covered by the core `qwen35` dialect (`hermes_json`) and is not duplicated: here it is rejected with a pointer.
  Odysseus has no literal ```` ```tool ```` tag; its fence tag is the tool name (```` ```bash ````).

## Not executed

In 0.1 only `read_file`, `ls`, `glob`, `grep` execute, inside the workspace given to the constructor.
Every other authorized tool returns `status: error`, `execution_error`, "not executed by the reference
adapter". `execute_tool_block` itself is deliberately not called: its admin check and MCP routing need
Odysseus app state. Not modeled: `tool_policy`/disabled tools, exact-approval replay, scoped approvals, and (new in the `execute_tool_block` path at a8c147b) request authority/delegated-credential pre-checks (`tool_execution.py:1654-1700`).

## Finding

At `2992bf6`, `tail_serve_output` was in the catalog but not in `TOOL_TAGS`, so the adapter answered `not_found`. At `a8c147b` it is registered (`tool_types.py:13`): every recorded tool is in `TOOL_TAGS`, and the call is `authorized` but not executed (only the four read-only tools run). Tests pin both facts; the `not_found` branch is covered by a test that removes a tool from `TOOL_TAGS` for its duration.

Re-pin facts (a8c147b vs 2992bf6, from re-extraction): 17 tools added (`block_sender`, `download_attachment`, `draft_email`, `draft_email_reply`, `extract_text`, `generate_image`, `get_weather`, `host_shell`, `inspect_media`, `manage_email_state`, `manage_research`, `pdf_extract`, `private_browser`, `scan_spam`, `search_emails`, `transcribe_media`, `youtube_tool`), none removed, 34 of the 71 old tools changed in some field (22 of them in their parameter schema). Recorded effects changed for `manage_endpoints`, `manage_mcp`, `manage_settings`, `manage_tokens`, `manage_webhooks`: `admin_change` before, `read_private` + `write_private` now. That is an artifact of recording each tool with an empty action: upstream now classifies those managers by action, and an empty action falls to its read/write fallback. The adapter never uses the recorded effects to decide; `decide` calls `capabilities_for_action` with the real arguments. A differential run of every tool across 8 configurations (admin, external context, delegated credential) gave identical decisions at both pins, except the `tail_serve_output` change and schema-dependent argument validity.

The recorded `catalog_digest` (`sha256:ad89a622...`) is `Catalog.computed_digest()` over the 88
recorded capabilities, so anyone can recompute it from the record with Core alone. The previous record
(`catalog.odysseus-2992bf6.json`, 71 tools, `sha256:4ec0d10e...`) stays in this directory unchanged only as the frozen input of the benchmark corpora
(`bench/tools/validate.py`); the adapter does not load it.

## Workspace and process-launch boundaries (#93)

Four separate questions, answered by four separate things:

1. **Selection** is `vet_workspace` (`src/tool_execution.py:1179`, what `/api/workspace/vet` returns): is this a real, non-sensitive directory? It accepts an ancestor of `ODYSSEUS_DATA_DIR` on purpose and refuses `DATA_DIR` itself and anything nested in it.
2. **Confinement** is Odysseus's per-path deny: inside an accepted workspace, files of server state are hidden one path at a time.
3. **Launch preflight** is `launch_preflight()` here: would the launch boundary (`guard_launch_workspace`, `src/agent_runtime/process_resources.py:379`) structurally accept this workspace for bash, python, host_shell or manage_bg_jobs? Read-only; asks the real upstream code.
4. **Authorization** stays with Odysseus at launch: tool permission, admin, delegated credentials, untrusted context, approvals. A positive preflight is none of these and not a claim that anything ran.

**Verified disagreement** (a8c147b, real run): a workspace that is an ancestor of `DATA_DIR` is accepted by selection but refused by the launch guard ("contains server control state"), so the workspace can be chosen and then every launch fails. A workspace nested inside `DATA_DIR` is the reverse: refused by selection, passed by the guard alone. The preflight runs both: Odysseus's own `vet_workspace` and launch guard, preceded by the adapter's equal/nested check against `DATA_DIR` (the same rule `vet_workspace` applies, repeated so the reason code can name the case). Upstream: odysseus-dev/odysseus#6651 (open, unfixed at a8c147b). INTERPLANE does not bypass or weaken either gate.

**Safe layout.** Keep `ODYSSEUS_DATA_DIR` disjoint from every agent workspace (siblings, not parent or child). The loader defaults it to a fresh temp directory only when unset; an operator value is always honoured. The adapter never moves, copies or rewrites existing state; the fix for a refused layout is the operator's.

| Code | Stage | Meaning | Remediation |
|---|---|---|---|
| `structurally_eligible` | preflight | Passed every structural check; not an authorization | Odysseus still decides at launch |
| `runtime_unavailable` | runtime | Odysseus not importable | Set `ODYSSEUS_SRC` |
| `not_a_process_tool` | request | Tool is not bash, python, host_shell or manage_bg_jobs | Read-only tools need no launch preflight |
| `process_tool_unavailable` | runtime | Tool missing from `TOOL_TAGS` or `TOOL_HANDLERS` | Use a revision that registers it |
| `host_api_unsupported` | runtime | No launch guard API (e.g. the former pin 2992bf6) | Treat launch as not ready |
| `workspace_missing` | selection | No workspace given | Name one |
| `workspace_invalid` | selection | Not an existing directory | Check the path |
| `workspace_inaccessible` | selection | No read/search access | Fix permissions or choose another |
| `workspace_is_server_state` | selection | Workspace is `DATA_DIR` | Choose another; keep `DATA_DIR` outside workspaces |
| `workspace_within_server_state` | selection | Nested inside `DATA_DIR` | Choose another |
| `workspace_rejected_by_host` | selection | `vet_workspace` refuses it | Choose another |
| `indeterminate` | launch_boundary | Boundary could not be sealed or inspected | Fix permissions or shrink the workspace, retry |
| `workspace_contains_server_state` | launch_boundary | Contains control state (the #6651 case) | Disjoint workspace or `ODYSSEUS_DATA_DIR` outside it |
| `workspace_aliases_server_state` | launch_boundary | Symlink or hardlink to server state inside | Remove the link or choose another |

Results carry fixed text only: no paths, no upstream exception text.

**Qualification matrix** (full suite `tests`: 117 tests, no skips; `tests/test_launch_preflight.py`: 24 of them, real upstream calls):

| Odysseus | Full suite | Preflight |
|---|---|---|
| a8c147b (catalog pin, 2026-10-07) | 117 passed, 0 skipped | full table: sibling and lookalike eligible; equals, nested, ancestor, other-control-path, symlink, hardlink, invalid, inaccessible denied |
| 2992bf6 (previous pin) | 113 passed, 4 failed (the three drift tests and the fresh-extraction check, which now describe a8c147b) | 24 passed: every workspace case reports `host_api_unsupported` (no `process_resources.py`) |

Execution stays fail-closed (only `read_file`, `ls`, `glob`, `grep` run). CI runs the full suite once, at `a8c147b`, with `set -o pipefail` and no skips allowed.

**Limitations.** The guard is pathname and inode based and not atomic (its own docstring): links can change after the check. The nested case relies on selection, not the guard. The guard walks the whole workspace tree, so very large workspaces cost time. A positive result is not an authorization. No shell, python or job execution was added to the adapter.

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
