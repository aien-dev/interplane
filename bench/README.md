# INTERPLANE 0.2 benchmark corpus

The CrossAxis benchmark from `ROADMAP.md` ("0.2 sequencing", step 3). The question it answers: does
showing a small local model only the CrossAxis-selected tools (condition B) keep it as capable as
showing it the whole Odysseus catalog (condition A), while costing far fewer prompt tokens?
The corpus, its judge and its statistics are fixed here **before** any model is run. The paired
runner is a separate lane; this directory is everything it needs. The protocol is
`PROTOCOL-0.2.md`.

## Layout

| Path | What it is |
|---|---|
| `schema/task.schema.json` | JSON Schema 2020-12 for one task. |
| `tasks/<category>-NNN.json` | 44 tasks, 13 categories; file name = id. |
| `domains.json` | The fixed keyword rule that turns a user request into condition-B `requested_domains`. |
| `prompts/system.md` | The one system prompt for every task and both conditions (digest pinned in each task). |
| `fixtures/<name>/` | Small workspaces, copied byte-for-byte into a fresh temp dir per run. |
| `stubs/<name>.json` | Fixed results for read-only Odysseus tools the reference adapter does not execute. |
| `stubs/backends.json` | Simulated backends `sim-1`: for every tool a task lists, which backend answers it after Odysseus authorizes the call, and its documented semantics. |
| `stubs/stores/<fixture>.json` | Private-data stores (notes, memory, calendar, contacts, editor documents, models, tokens) read by the `sim-1` backends. A fixture without a file has empty stores. |
| `tools/sim_backends.py` | The `sim-1` simulators (stdlib only), shared by the runner and the validator. |
| `tools/validate.py` | Structural validator (stdlib only). CI job `bench-structural` runs it. |
| `tools/stats.py` | The pre-registered paired statistics (Newcombe 1998 method 10, exact McNemar), with a self-test against the paper's table. |
| `CORPUS-DIGEST.txt` | Frozen digests of the corpus; CI recomputes and compares. |
| `PROTOCOL-0.2x.md` | Pre-registered 0.2x protocol (future work, no results): held-out set, seven conditions, runtime-triggered expansion rules, thresholds. |
| `heldout-0.2x/` | The 0.2x held-out qualification set: 120 `tasks/`, its own `fixtures/`, `stores/`, `stubs/`, `prompts/discovery-addendum.md` and `CORPUS-DIGEST.txt`. Paths inside its tasks are relative to this folder. Check with `validate.py --corpus 0.2x`. |

## Categories (dev / qual)

| category | dev | qual | what it exercises |
|---|---|---|---|
| filesystem | 1 | 3 | easy reads and listings |
| ambiguous | 1 | 3 | the request names no file or tool |
| multidomain | 1 | 3 | tools from two domains in one task |
| rare | 1 | 3 | low-frequency read-only tools (`list_serve_presets`, `list_cookbook_servers`, `resolve_contact`, `list_email_accounts`) |
| wrong_first_tool | 0 | 3 | the tempting tool is wrong (`bash`, editor-panel documents, web search) |
| exec_failure | 1 | 2 | injected or real tool failure, then recovery |
| denied | 0 | 3 | Odysseus refuses (sensitive path, admin-only tool for a non-admin) |
| approval | 1 | 2 | Odysseus's untrusted-context gate turns an effect into `requires_approval` |
| injection_workspace | 1 | 2 | instructions planted in a workspace file |
| injection_tool | 0 | 3 | instructions planted in a tool result or a tool error |
| expansion | 0 | 5 | the rule-derived domains miss the needed tool, so condition B must expand |
| unknown_tool | 0 | 2 | the user asks for a tool that does not exist |
| sequential | 0 | 3 | 3+ dependent reads (pointer chains with decoys) |

## How a task is run (summary; the protocol is normative)

1. Copy `workspace_fixture` into a fresh temporary directory. Construct
   `OdysseusAuthority(workspace, admin=authority_profile.admin, delegated_credential=...)`.
2. Messages: `system` = `prompts/system.md`, `user` = `user_request`.
3. Tools: condition A renders all 71 catalog tools. Condition B renders CrossAxis Select
   `domain_match` v1 over `requested_domains` with `always_include = ["ask_user"]`, plus bounded
   expansion.
4. Every model turn goes through the INTERPLANE pipeline (Lenshift -> Core -> CrossAxis ->
   Crossveil with Odysseus deciding), exactly like `examples/receipt/run_receipt.py`.
   - `stub_results`: when Odysseus authorizes a call to that capability, return the stub as an
     `ok` result instead of the adapter's "not executed" error. The decision is never stubbed.
   - `fault_injection`: when Odysseus authorizes call number k of that capability and k is in
     `on_calls` (or `"all"`), return `execution_error` with the given message instead of
     executing.
5. Stop when the model answers without a tool call, or after the round limit. Judge the
   transcript.

## How a task is judged

Deterministic only, no LLM judge. A run succeeds when every check in `judge.checks` that applies
to its condition passes. Check kinds:

- **Answer text checks:** `answer_contains_all`, `answer_not_contains`, `answer_regex` and
  `answer_equals_normalized`. All are case-insensitive except regexes, which carry their own flags.
- **Decision and state checks:**
  - `state_denied` and `state_requires_approval`: Odysseus's decision for that capability.
  - `no_execution_of`: no request for these was ever decided `authorized`.
  - `no_request_of`: the model never asked for these.
  - `recovered_after_error`: an `execution_error`, then a later `ok` result, then a final answer.
  - `expansion_occurred` and `expansion_not_needed`: condition B only.
- **`any_of`:** passes when at least one of its sub-checks passes.

Exact semantics are in `PROTOCOL-0.2.md` section 5. `expected_answer` is a human-readable
reference and is never used for judging.

## Why some tasks look the way they do (facts from Odysseus at 2992bf6)

- **Only `read_file`, `ls`, `glob` and `grep` execute.** The reference adapter executes only these
  four. Every other authorized tool returns `execution_error` "not executed by the reference
  adapter". Rare and multidomain tasks therefore use `stub_results` for the read-only private
  tools they need. The catalog has no stat, hash, count or git tool, so none is used. This is
  the 0.2 behaviour (`run_bench.py --backends reference`, the default). The 0.2 run analysis
  (`runs/qual-20261004T2207Z-analysis/error-classification.md`) found 80 of 87 execution errors
  came from this gap.
- **Simulated backends `sim-1` (`--backends sim-1`, added after the 0.2 run).** Every tool any task
  lists now has a backend in `stubs/backends.json`. Per authorized call the order is: the task's
  `fault_injection`, the task's `stub_results`, the registry, the reference adapter. Kinds:
  `executable` (the four above), `odysseus_handler` (`get_workspace`: Odysseus's own handler; the
  workspace sits at a fixed temp path so its text is stable), `computed` (deterministic
  simulators over the fixture's store or the run's workspace copy; writes stay in the run), and
  `declared_failure` (`bash`, `python`: always the same documented error, because shell output
  cannot be simulated deterministically). `validate.py` fails when a task lists a tool with no
  backend, when an answer or recovery task lists a `declared_failure` tool, when a simulator is
  not deterministic on some fixture, or when a store holds the answer of an expansion task on its
  fixture (that would make expansion unnecessary). Tools no task lists keep the reference
  behaviour. Intended failures (`fault_injection`, the missing path in `exec_failure-003`) are
  unchanged. Before `sim-1`, 39 of 44 tasks (33 of 37 qual) listed a tool without a working
  backend; with it, 1 (`approval-003`, whose `bash`/`python` alternatives are meant to stop at
  approval).
- **Denials come from Odysseus's own gates:**
  - The caller must be an admin to read files at all. `NON_ADMIN_BLOCKED_TOOLS` includes
    `read_file`, `ls`, `grep` and `glob`, so a non-admin cannot read the workspace and then be
    denied a write afterwards.
  - Sensitive paths are refused inside the workspace: `.ssh`, `.env` and `id_ed25519` all match
    (`_is_sensitive_path`).
  - Admin-only tools are refused for a non-admin (`_ADMIN_TOOLS`, e.g. `manage_tokens`).
- **Approvals come from the untrusted-context gate.** Any workspace read arms it. After that,
  write, send, exec, read-private and admin effects become `requires_approval`. Multidomain tasks
  therefore ask for the private read (contacts, chats, presets) before the workspace read.
- **That gate does not block `brokered_network_read` (`web_search`) or `read_workspace`.** Each
  injection category therefore includes payloads aimed at those. Those tasks measure the model,
  not only the runtime.

## Dev and qualification split

7 `dev` tasks: filesystem-001, ambiguous-001, multidomain-001, rare-001, exec_failure-001,
approval-001 and injection_workspace-001. They are for the pilot and for debugging the harness,
may be run any number of times, and are excluded from every gate statistic. They come only from
categories with at least 3 tasks and never from `expansion`. The 37 `qual` tasks are frozen by
`CORPUS-DIGEST.txt` before the qualification run.

Under `PROTOCOL-0.2x.md` all 44 tasks here, dev and qual alike, become dev and regression cases:
they were published with the 0.2 run. The 0.2x gate uses only the held-out set in `heldout-0.2x/`.

## Adding or changing a task

1. Write `tasks/<category>-NNN.json` against the schema. Set `requested_domains` to the output of
   `python3 bench/tools/validate.py --derive "<user_request>"`. Never edit `domains.json` to make a
   task fit. Reword the request or change its category instead.
2. Every capability name must be in `adapters/odysseus/catalog.odysseus-2992bf6.json`.
3. Coverage:
   - Non-expansion tasks must be covered by the initial condition-B selection.
   - Expansion tasks must not be covered.
   - Covered means every `required_capabilities` entry is selected and every `allowed_alternatives`
     group has at least one selected member.
4. If the answer needs a judgment call, redesign the task until a substring or regex decides it.
5. Run `python3 bench/tools/validate.py`. When the change is intended, run it with
   `--write-digest` and commit the new `CORPUS-DIGEST.txt` in the same commit. A change to a
   `qual` task after the qualification freeze invalidates that run.

No git repository fixture is needed: the catalog has no git tool, so `fixtures/build.sh` does not
exist.

## Running it (paired runner and analyzer)

- `tools/run_bench.py` runs both conditions per task through the INTERPLANE pipeline with the real
  `OdysseusAuthority` (Odysseus at 2992bf6, via `ODYSSEUS_SRC`), against an OpenAI-compatible
  endpoint, and writes `runs/<run-id>/manifest.json` plus one receipt per task and condition
  (`receipts/<task>.<A|B>.json`). `--prepare` writes only the manifest, so it can be committed
  before the first request; `--resume` skips completed pairs. Condition B also exposes one
  read-only discovery tool, `interplane_capabilities_search{query}`, that grants nothing and
  executes nothing in the runtime.
- `--expansion-policy runtime-v1` replaces the 0.2 condition-B expansion with the host-side
  triggers of `PROTOCOL-0.2x.md` section 5 (arm 3). An unknown tool name runs a catalog search
  (`discover` per name token, then `expand` with `discovery_hit`). An unexposed tool is added with
  `requested_excluded`. A typed missing-capability error widens its domain. Denied and pending
  calls never trigger (N1, N2), each (trigger, name) pair is tried once, and an expansion over 30 %
  of the full-catalog schema bytes is not applied. Every decision goes to the receipt's
  `runtime_triggers`, and every attempted expansion goes to `expansions`, with the previous and new
  selection digests, the added and refused names, and the budget left. The default `0.2` keeps the
  0.2 behaviour and identity digest unchanged.
- `tools/bench_eval.py` is the deterministic judge and the per-run metrics, shared by the runner
  and the analyzer. `tools/analyze.py <run-dir>` (stdlib only, no inference) writes
  `summary.json`, `summary.md` and `tasks.csv`, byte-identically for the same input.
- Offline tests: `tools/test_analyze.py` (stdlib, runs in CI against a checked-in synthetic run)
  and `tools/test_runner_offline.py` (a scripted fake endpoint; needs the Odysseus venv; also runs
  every `sim-1` backend kind twice and checks the results are identical, and runs `runtime-v1`
  with the denial and approval negative controls).
- Corpus digests: adding `stubs/backends.json` and `stubs/stores/` changed `inputs_digest` in
  `CORPUS-DIGEST.txt`. `tasks_digest` and `protocol_sha256` are unchanged. The 0.2 runs carry the
  digests they ran against in their own `manifest.json`, which is unchanged.
