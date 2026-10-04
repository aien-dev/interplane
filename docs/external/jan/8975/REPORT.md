# Jan #8975: duplicate MCP tool names advertised twice

Project: janhq/jan, default branch pinned at 14a720628f9592c8e60f7e081ef733076db8a6a9 (2026-10-02).
Issue: https://github.com/janhq/jan/issues/8975 (open, no assignee, no linked PR or branch as of 2026-10-04).
Fork branch: aien-dev/jan `fix/mcp-duplicate-tool-names-8975` (test 27a66f0, fix c13dfa0, tidy 40b96aa).
Evidence level: reproduced (unit level), fixed, tested. Not submitted. Drake is the submitter.

## Observed
- `assemble_tool_array` (src-tauri/src/core/agent/upstream.rs:919 at the pinned SHA) flattens every server's tools into the advertised array and inserts each into `tool_to_server`, a `HashMap<String,String>`. Two servers exposing `search` give two `search` entries in the array and one map entry (last server by sort order wins).
- The repo's own test `a_tool_name_exposed_by_two_servers_routes_the_same_way_every_run` asserted `advertised_names == ["search","search"]` (upstream.rs:2134 at the pinned SHA), i.e. the duplicate was recorded as known behavior.
- Reach (observed by grep at the pinned SHA): every Rust request path builds its tool list via `collect_mcp_openai_tools` (upstream.rs:961, which ends in `assemble_tool_array`, :1030): agent loop loop.rs:2964 and :3313, local API proxy proxy.rs:1303. So the fix covers all of them.
- The web UI path (web-app/src/lib/custom-chat-transport.ts:1098) keys tools by name and warns on collision, so it cannot send duplicates.

## Reproduction (exact commands, aarch64 Linux, headless, no model, no GPU)
```
cd ~/workspace/external/jan && git checkout 27a66f0   # test only, on top of 14a7206
cd src-tauri
nice -n 19 cargo test --locked --no-default-features --features cli --lib -j4 \
  the_advertised_array_has_unique -- --test-threads=1
```
Result before the fix (observed): FAILED, `left: ["read", "search", "search"] right: ["read", "search"]` "duplicate function names on the wire". Receipt (observed, red.log): `left: [\"read\", \"search\", \"search\"]`, `right: [\"read\", \"search\"]`, `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2316 filtered out`.

## Root cause
Proven by the red test plus source read: no uniqueness step between flatten and push (upstream.rs:919-940 at the pinned SHA). The provider-side effects (strict provider rejects, lenient provider keeps one) are from the issue text and are hypothesized here; not exercised against a real provider.

## Fix (smallest)
After sorting, record the winning server per tool name (last in sort order, which is exactly what `tool_to_server` already did), advertise only the winner's definition, warn naming both servers. Order stays sorted and byte-stable, nothing depends on HashMap iteration, no listing is dropped on server hiccups (reuse path untouched). The old test now asserts `["search"]`.
Design note for the maintainers: the issue lists three options (namespace, reject-and-warn, first-wins) and says "Not decided here". This branch keeps today's routing winner and removes the shadowed duplicate; it does not rename tools (no breaking change). If maintainers prefer namespacing, the new test (unique names, array matches routing table) still applies unchanged.

## Regression
Full lib suite in cli mode: 2316 passed, 1 failed. Receipt (observed, green.log): `test result: FAILED. 2316 passed; 1 failed`, the one failure being the unrelated test named below. The one failure, `core::cli::tui::tests::reload_system_prompt_reads_agents_md_fallback`, also fails on the unmodified base 14a7206 on this machine (observed), so it is unrelated (likely an AGENTS.md in a parent directory of this box). `cargo fmt --check` shows no diff in lines touched by this change; other files and lines already differ at the base.
Command: `cargo test --locked --no-default-features --features cli --lib -- --test-threads=1` (the exact CI command, jan .github/workflows/rust-check.yml:121).

## Scope
Cross-server collisions only. Two edits in one file: `assemble_tool_array` and one test (+ old test assertion). No API, schema or naming change.

## Design choice, stated plainly
This is the issue's "single winner" option, not "reject and warn": the warning is only a log line, so a shadowed tool still disappears from the model's view without a user-visible error. Maintainers may prefer namespacing or a visible error. Also not covered: one server listing the same name twice (invalid per MCP, not addressed).

## Not done / waiting
- No end-to-end run through `jan` against a real or fake provider with two MCP servers. A fake OpenAI-compatible endpoint recording the request body (no model) could add that; needs a built `jan` CLI plus two stdio MCP servers. Optional follow-up, no INFERENCE OK needed.
- Upstream submission: Drake's button (buttons row 4).

## INTERPLANE lesson (one generic invariant)
A tool namespace must be injective on the wire: the advertised tool list must contain each callable name once, and the definition the model reads must be the one the call routes to.
