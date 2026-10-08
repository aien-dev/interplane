# CURRENT_STATE: live boundaries INTERPLANE must fit around

Phase 0 audit, 2026-10-04. Every claim below was read from executable source at the pinned
commits, not from READMEs. Two scouts read the code; the orchestrator re-verified the claims the
design depends on (marked **verified**). Nothing was executed; "implemented" means the code exists
and is wired into a live path.

Pinned sources:

| Project | Repo | Commit | Default branch | License |
|---|---|---|---|---|
| Odysseus | odysseus-dev/odysseus | 2992bf6 (2026-10-01) | dev | AGPL-3.0 |
| AIEN sovereign core | aien-dev/aien-sovereign-core | 9b5e6e8 (2026-10-08 adapter pin; was 7580039 at the 2026-10-04 re-audit) | main | AGPL-3.0-or-later |

VAC M3b (2026-10-08): the adapter still pins 9b5e6e8 for its crates. The fix-the-test slice gate (adapters/aien/scripts/test-fix-the-test.sh) was run against a daemon built from sovereign-core 47f1014 (#346: `ComposeRecall` reports `compose_native` and `omega_sha`; the daemon prints `Compose: native (omega <sha>)`), CPU only, native compose linked from omega 6c6180c. Receipt: adapters/aien/evidence/fix-the-test-2026-10-08/receipt.json. The model turn of that run is scripted.
| AIEN aegis-runtime | aien-dev/aegis-runtime | f4e8709 (2026-10-04 re-audit: still origin/main) | main | AGPL-3.0-or-later |
| AIEN protocols | aien-dev/aien-protocols | 3a4cdbe (2026-10-04 re-audit: still origin/main) | main | AGPL-3.0-or-later |
| AIEN architecture | aien-dev/aien-architecture | 6c95697 (2026-10-04 re-audit: still origin/main) | main | AGPL-3.0-or-later |
| Qwen3.5 chat template | Qwen/Qwen3.5-9B `chat_template.jinja` | sha256 a4aee8af... (153 lines) | main | Apache-2.0 |

Classification key: implemented / partially implemented / merged but unused / experimental /
proposed / deprecated / abandoned / absent.

## 1. Odysseus (Python)

| Concern | State | Where (file:line at 2992bf6) |
|---|---|---|
| Agent loop (model -> tool -> result -> model) | implemented | `src/agent_loop.py:3420` `stream_agent_loop`; round loop `:4773`; default `MAX_AGENT_ROUNDS = 50` (`src/agent_tools/__init__.py:74`), user setting clamped 1..200 (`routes/chat_routes.py:2323`); tool-call budget `:5633`; `rounds_exhausted` event `:6314` |
| Native tool calls | implemented | streamed `delta.tool_calls` accumulated in `src/llm_core.py:3050-3069`; converted by `function_call_to_tool_block` (`src/tool_schemas.py:1370`), which rejects missing required args (`:1398`), rejects names not in `TOOL_TAGS` (`:1411`), passes `mcp__*` through (`:1403`) |
| Text tool-call parser | implemented, many dialects | `parse_tool_blocks` (`src/tool_parsing.py:1287`): fenced ```` ```tool ````, `[TOOL_CALL]`, `<tool_call>`/`<function_call>` with `<invoke name>`/`<parameter name>` body or bare Hermes JSON body (`_parse_json_tool_call_body` `:940-972`, **verified**: strict object with string `name`, `arguments` must be an object), `<tool_code>`, StepFun tokens, DeepSeek DSML, Gemma `<|tool_call|>call:name{...}`, `<function_model>`, raw OpenAI JSON in text |
| Qwen3.5 official XML (`<function=NAME><parameter=K>`) | **absent** (**verified**: `grep 'function=' src/tool_parsing.py` matches nothing) | issue #6412 (Qwen3-Coder form) has open PR #6413 (0 reviews). Not duplicated here. |
| Tool result reinjection | implemented | native: assistant `tool_calls` + one `role:"tool"` message per call (`src/agent_loop.py:3038-3071`); text mode: results wrapped by `untrusted_context_message` (`:3095-3120`); `metadata.trusted/source` on results (`:3082-3090`) |
| Tool registry | implemented, hardcoded | `FUNCTION_TOOL_SCHEMAS` OpenAI function format (`src/tool_schemas.py:34+`, ~60 tools: `bash{command}`, `python{code}`, `read_file{path,offset,limit}`, `write_file{path,content}`, `edit_file{path,old_string,new_string,replace_all}`); name set `TOOL_TAGS` (`src/agent_tools/__init__.py:79`); if/elif dispatch `src/tool_execution.py:965-1357`; registry refactor is proposed only (issue #4277) |
| Effect classification | implemented | `src/tool_capabilities.py:21-34` `ToolEffect` enum (**verified**): read_public, read_workspace, read_private, write_workspace, write_private, execute_code, brokered_network_read, network_egress, external_side_effect, ui_side_effect, admin_change, destructive, user_interaction. Unknown tools fail high (`:298`) |
| Authority boundary | implemented, layered | (1) `tool_policy.blocks()`/disabled tools (`src/agent_loop.py:5659-5682`, `src/tool_execution.py:1050`); (2) untrusted-context gate `ToolRunSecurityContext.decision_for -> ToolGateDecision{allowed, reason}` (`src/tool_capabilities.py:569`, `:654-684`, **verified**); (3) sealed exact approvals `PendingToolApproval`/`ExactToolApproval` digest-bound to owner, session, run, tool, content, workspace (`src/tool_approvals.py:104-142`), replay verified at `src/agent_loop.py:4526`; (4) approval scopes SINGLE_ACTION/TASK/CHAT_SESSION, HMAC-signed (`src/tool_approval_scopes.py:45-151`); (5) admin/owner checks (`src/tool_execution.py:539,1064`, `src/tool_security.py:42`); (6) path confinement (`src/tool_execution.py:357-460`) |
| Decision states observable today | implemented | allowed; blocked by policy; denied by gate -> pending approval (`approval_required`/`ask_user` events `:5766`); approved replay. Maps to Crossveil AUTHORIZED / REJECTED(policy) / REQUIRES_APPROVAL / DENIED |
| Provider abstraction | implemented | `_detect_provider` host-based (`src/llm_core.py:966-1002`); OpenAI-compatible `stream_llm` `:2559`; Anthropic `:1527`; Ollama native `:731`; llama.cpp/LM Studio/vLLM extras `:1004-1027`; model identity is a plain string plus endpoint row with `supports_tools` flag |
| Qwen3.5 handling | partially implemented | thinking detection by `qwen3` substring (`:1439-1450`); Ollama `think:false` (`:2654`); `reasoning_content`/`reasoning`/`thinking` delta keys (`:3190`); system-message consolidation "Qwen3.5 rejects non-first system" (`:2599`); no `chat_template_kwargs` sent |
| MCP | implemented | client over stdio/SSE/streamable-HTTP (`src/mcp_manager.py:168-258`); tools namespaced `mcp__{server}__{tool}` with server `input_schema` (`:570-602`); unknown MCP tools are "unknown/high-impact" after external context (`src/tool_capabilities.py:298-310`) |
| Tracing / provenance | partially implemented | per-run `run_id` uuid (`src/tool_capabilities.py:621`); no `trace_id`, no request-id middleware; tool calls logged with first 200 chars of response; URL redaction only, no argument redaction |
| Ajax (model) | absent (**verified**) | only a CDN path in a bundled JS file |
| Real captured model fixtures | absent | parser tests use hand-written strings, e.g. `tests/test_tool_parsing_hermes_json.py:15` `<tool_call>\n{"name": "bash", "arguments": {"command": "mkdir -p agent-test"}}\n</tool_call>` |
| Contribution rules (**verified**) | - | PRs target `dev`; one fix per PR; large features need an issue first; **LLM agents are asked to open an issue instead of a PR; bulk agent PRs are closed** (`CONTRIBUTING.md:77`) |

Open upstream issues relevant to local-model tool interop (2026-10-04): #6474 (llama.cpp `-hfr`
is the repo alias, not the file flag; **verified** at `src/tools/cookbook.py:465`; no PR), #6477
(raw tool listing appended after answer; no PR), #6459 (PR #6461 open), #6412 (PR #6413 open),
#6423 (PR #6420 open).

## 2. AIEN (Rust)

There are **two unconnected agent stacks** and **three unconnected policy mechanisms**.

| Concern | State | Where |
|---|---|---|
| Agent loop A | implemented | `aegis-runtime/src/agent.rs:95-240` `AgentEngine::execute_task`; `tools = skills.to_openai_tools()` (`:103`, `skills.rs:87-108`); request body `tools` (`inference.rs:246`); native `tool_calls` read (`inference.rs:68,258`); `max_turns` loop (`:130`), gateway default 8 (`gateway.rs:147`); reinjection: assistant `tool_calls` + `role:"tool"` message (`agent.rs:168-205`)  <br>**2026-10-04 re-audit:** architecture migration plan `:146` and ADR 0016 `:1485-1493` declare it legacy orchestration, oracle then retire (R16); not canonical. |
| Agent loop B | implemented | `aien-sovereign-core/crates/aien-cli/src/main.rs` REPL (`max_tool_steps = 10`), subagents (`subagents.rs:148`, depth 3); tools described as prose in the system prompt (`client.rs:100-160`); parser `extract_tool_calls` regex `(?s)<tool_call>\s*(.*?)\s*(?:</tool_call>|$)` (`client.rs:383`) plus fenced-JSON fallback and brace/quote repair (`:350-380`); reinjection as a **user-role** `<tool_response name="...">` message (`main.rs:327-330`)  <br>**2026-10-04 re-audit:** no ADR or plan declares it canonical or retired (undeclared). |
| Text parser A | implemented | `parse_structured_tool_calls` (`aegis-runtime/src/inference.rs:670-742`): ```` ```json ```` block, `<tool_call>` tags, standalone `{...}`; accepts `name|tool`, `arguments|params`; mints `call_<uuid>` ids |
| OpenAI-compatible serving | implemented, not tool-aware | `aegis-runtime/src/gateway.rs:672` `POST /v1/chat/completions`; `OpenAiChatRequest` has no `tools` field (`:173-179`) |
| OpenAI-compatible consuming | implemented | `HttpInferenceBackend` default `http://127.0.0.1:18006/v1/chat/completions` (`inference.rs:152`) |
| Tool definitions A | implemented | `SkillDefinition{name, description, parameters_schema, advertised}` (`skills.rs`); builtins `bash_eval{command,cwd}` (allowlisted commands only), `read_file{path}`, `write_file`, `list_dir`, `git_status`; allowlist `ALLOWED_SKILLS` (`enforcement.rs:14-24`) |
| Capability descriptor | merged but unused | `aien-capability`: `ToolDescriptor{name, effects: ToolEffects, schema_digest}` (`tool.rs:7-11`, **verified**); `ToolEffects` u32 bitset PURE, READ_FILESYSTEM, READ_NETWORK, SPAWN_PROCESS, WORLD_MUTATION, LOCAL_EPHEMERAL, EXTERNAL_WRITE, EXTERNAL_IRREVERSIBLE, SECRET_BEARING (`effects.rs:5-14`, **verified**); `EffectClass` enum; **no crate depends on it** |
| aien-mcp | merged but unused | rmcp 3.4 client+server, generic transport; `McpBroker::admit`, `SpeculativeLane` refuses non-speculation-safe tools, `stage_effect_intent`, `EffectLane::execute_effect` requires `AuthorizedEffect`, which has **no production constructor** (`effect.rs:35-39`, test-only mint `:76-93`); no crate depends on it  <br>**2026-10-04 re-audit:** unchanged at origin/main 0bcd558 (effect.rs:40-47, 76-93); still no crate depends on it; adapter builds and tests (10 pass) against it. |
| Authority A (live) | implemented | `SkillRegistry::execute_gated` -> `enforcement::probe_gate` -> `ProbePolicyGuard::gate_skill` -> `pre_dispatch_check(skill_name, &Value) -> Result<(), String>` (`enforcement.rs:31`, **verified**; allowlist, JSON-object args, `admit_local_command` for bash) + `aien-probe` safety threshold 0.5; **no approval-required state reachable**  <br>**2026-10-04 re-audit:** architecture migration plan `:147` marks it 'Not authority', Replace by capability root + Effect Broker (ADR 0005/0016). |
| Authority A (typed, unproduced) | merged but unused | `DoctrineDecision{Allow, AllowRestricted(Vec<String>), RequireApproval, Deny(String), Contain(ContainmentLevel)}` (`defense/decision.rs:14-22`, **verified**, serde tag `status`/`details`); `ExecutionAuthority` trait has no implementer; `ApprovalStatus{Pending,Approved,Rejected,Expired}` + `approvals` table exist  <br>**2026-10-04 re-audit:** `ExecutionAuthority` still has zero implementers (`execution/broker.rs:16-17`); its `ActionRequest` is aegis-local, not the aien-protocols one. |
| Authority B (live) | implemented | `SafetyEngine::evaluate(tool, &Value) -> SafetyDecision{Allow, AskUser(String), Deny(String)}` (`aien-cli/src/safety.rs:8-12,76`, **verified**); `AskUser` prompts at the terminal (`tools.rs:85-92`)  <br>**2026-10-04 re-audit:** still private: `mod safety;` at aien-cli `main.rs:14`, `lib.rs` empty. Also a fourth in-repo `ProbePolicyGuard` copy in `spark-mail-rs/src/effect.rs:116`. |
| spark-aegis / aien-security | implemented, not a capability authorizer | containment, scanning, audit; `enforce_network_purpose` only forbids Telemetry |
| Effects boundary | experimental, journal only | `aien-runtime/src/world.rs:9-26` `EffectIntent{WriteFile, ExecuteCommand, SendMessage, PublishGit}` recorded into `WorldManifest.effect_log`; no executor |
| Cortex | implemented, off the agent path | `cortex-rs` HTTP routes; CLI reaches it over HTTP; aegis-runtime skills `cortex.search` report "Unavailable" |
| Tracing / provenance | partially implemented | per-request `request_id` uuid (`aegis-runtime/src/inference.rs:375`); `ActionReceipt{receipt_id, action_id, success, output, error, execution_digest, completed_at}` (`aien-protocols/crates/aien-action-protocol/src/lib.rs:51-59`); `ControlEnvelope{protocol_version, request_id, operation_id, operator_session}` (`aien-runtime/src/control.rs:65-71`); SHA-256 digests in aien-capability, BLAKE3 in spark-aegis and aien-proof |
| aien-protocols schemas | implemented as serde types, no JSON Schema files | `CapabilityGrant`, `ContainmentState`, `ActionRequest`, `ActionAuthorization`, `ActionReceipt` (aien-action-protocol); `EventEnvelope` (aien-event-protocol); `InferenceRequest/Response` (aien-inference-protocol); no intent/decision/tool-call message type; `VERSIONING.md`: semver, major 0 may change wire formats |
| ADRs | accepted | 0003 tools vs skills; 0004 MCP is a compatibility layer (canonical at `aien-mcp`); 0005/0007 irreversible effects only as EffectIntents until a world wins; 0016 resident reaction architecture (AEGIS contributes capability policy, only AIENOS root mints grants); 0017 ARGUS never authorizes. None mention a model adapter protocol or Ajax  <br>**2026-10-04 re-audit:** ADR 0016 (`:54,:359,:1504`): the authority root is the native AIENOS capability authority; AEGIS decides no policy. See docs/audits/aien-authority-reaudit-2026-10-04.md. |
| Ajax | absent (**verified**) | only tokenizer vocabulary entries |
| Real captured model fixtures | absent | hand-written strings only, e.g. `aegis-runtime/src/inference.rs:821` `<tool_call>{"name": "bash_eval", "arguments": {"command": "ls"}}</tool_call>` |

## 3. Qwen3.5 (model dialect)

Official `Qwen/Qwen3.5-9B` chat template (fetched 2026-10-04, **verified**, lines 53, 105-137, 149-152):

- The assistant emits `<tool_call>\n<function=NAME>\n<parameter=KEY>\nVALUE\n</parameter>\n...\n</function>\n</tool_call>`. Parameter values are raw text that may span lines; the template serializes non-string JSON values with `tojson`.
- Optional natural-language reasoning may precede a call, never follow it.
- Tool results are rendered as `role: tool` content wrapped in `<tool_response>\n...\n</tool_response>`.
- Thinking is delimited by `<think>...</think>` before the content; with `enable_thinking=false` the template pre-fills an empty think block.
- This is **not** the Qwen3 Hermes JSON form (`<tool_call>{"name":..,"arguments":..}</tool_call>`). Serving engines (vLLM `qwen3_xml` parser, llama.cpp, Ollama) convert it to OpenAI `tool_calls`, and community reports show template drift and mid-thought calls. Both the XML form and the engine-converted OpenAI form are therefore real inputs.

Neither AIEN nor Odysseus parses the official Qwen3.5 XML form today.

## 4. Ajax

Announced 2026-10-02 as a Qwen3.5-9B fine-tune for Odysseus. As of 2026-10-03 the official page
says "when it's ready"; as of 2026-10-04 no model card, weights, license, tokenizer or chat template
are published (Hugging Face search for "ajax" returns unrelated repositories; the Odysseus code has
no Ajax provider). **Status: unavailable. No Ajax dialect is implemented; the dialect interface is
reserved (see `spec/LENSHIFT.md`).**

## 5. What this means for the design

1. Both runtimes already speak OpenAI `tool_calls` natively and both also text-parse `<tool_call>`
   JSON. The OpenAI dialect is the shared floor; the Qwen3.5 XML dialect is the first real gap.
2. Both runtimes classify effects (Odysseus `ToolEffect` strings, AIEN `ToolEffects` bits). The
   vocabularies differ; INTERPLANE must carry them as typed runtime extension metadata, not unify them.
3. Both runtimes have a three-way outcome today: allowed, denied, needs user approval (Odysseus
   sealed approvals; AIEN `SafetyDecision::AskUser`). AIEN's live aegis gate is two-way.
   Crossveil must represent all three without becoming the policy.
4. Neither runtime has a trace id. Odysseus has `run_id`; AIEN has `request_id`. Vectorveil
   needs `trace_id` + `message_id` + `request_id` and must accept runtime-local ids as extensions.
5. Tool results re-enter the model differently (Odysseus `role:tool`; AIEN CLI user-role
   `<tool_response>`; Qwen3.5 template `role:tool` + `<tool_response>`). RelayLine must define the
   result event, and Lenshift must render it per dialect.
6. Nothing in INTERPLANE may mint authority: AIEN's own `AuthorizedEffect` cannot be minted in
   production, and Odysseus approvals are digest-bound and server-signed.
