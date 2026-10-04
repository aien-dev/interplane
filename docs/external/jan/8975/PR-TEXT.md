Title: fix(agent): advertise one definition per MCP tool name

Target: janhq/jan, base `main`, head aien-dev/jan `fix/mcp-duplicate-tool-names-8975`.

## Describe Your Changes

- `assemble_tool_array` now advertises each MCP tool name once. When two servers expose the same name, only the definition from the server that `tool_to_server` already routes to is sent (last in sort order, unchanged), and a warning names both servers.
- Added `the_advertised_array_has_unique_names_and_matches_the_routing_table` (fails on main with `["read","search","search"]`) and updated the earlier collision test, which had asserted the duplicate.
- This is the "single winner" option from #8975 (the warning is a log line only, so a shadowed tool still silently disappears from the model's view). Cross-server collisions only; a server listing one name twice is not handled.
- No renaming, so saved prompts, permission lists and the byte-stable array order from #8974 are unaffected. Namespacing (option 1 in #8975) would be a separate decision.

## Fixes Issues

- Closes #8975

## How it was checked

`cd src-tauri && cargo test --locked --no-default-features --features cli --lib -- --test-threads=1`: 2316 passed, 1 failed. The failure (`reload_system_prompt_reads_agents_md_fallback`) also fails on the base commit on my machine and is unrelated.
Not run end to end against a live provider.

## AI disclosure

This change was prepared with an AI assistant (Claude Code, Sonnet 5.5) and reviewed by the submitter.

## Self Checklist

- [x] Added relevant comments, esp in complex areas
- [ ] Updated docs (for bug fixes / features)
- [ ] Created issues for follow-up changes or refactoring needed
