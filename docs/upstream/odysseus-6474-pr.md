# Upstream PR draft: odysseus-dev/odysseus#6474 (human submits)

Prepared by INTERPLANE 0.2 lane L1. Nothing here was opened or pushed upstream.

## Verified state (2026-10-04)
- origin/dev HEAD: `2992bf6d368a11472323e47d3bfed91e79cefc6b` (2026-10-01).
- Branch `fix/cookbook-llamacpp-hf-file` on fork `aien-dev/odysseus`: one commit `80f025d7a864f54a6bd767e4c78b338fb7242edb`, parent is origin/dev HEAD (no rebase needed).
- Issue #6474: OPEN. No upstream PR references 6474 (`gh pr list --search 6474`: none). `--search hf-file` returned only unrelated cookbook PRs (#6369, #6416, #6482, ...).
- origin/dev `src/tools/cookbook.py:465` still emits `-hfr {siblings[0]}`: not yet fixed.
- Diff is only `src/tools/cookbook.py` (+2/-2) and new `tests/test_cookbook_default_launch_cmd.py` (+25). Nothing INTERPLANE-related.
- Clean cherry-pick onto origin/dev in a scratch worktree (removed afterwards). Regression test: 2 passed with the fix; with the old cookbook.py the main test fails (1 failed, 1 passed), so it is a real regression guard. Run with the Odysseus venv (`~/workspace/odysseus/.venv`), `ODYSSEUS_DATA_DIR` pointed at a temp dir.

## Title
fix(cookbook): pass the GGUF filename to llama-server with --hf-file (#6474)

## Body
**Problem.** `_cookbook_default_launch_cmd` builds `llama-server -hf <repo> -hfr <file.gguf> ...`. On current llama.cpp `-hfr` is an alias of `-hf` (the repo flag), so the filename is parsed as a second repo and llama-server exits with "invalid HF repo format". Serving a GGUF repo from the cookbook fails. (Reported in #6474.)

**Fix.** Use the file flag, `--hf-file`, for the GGUF filename. The no-GGUF-sibling path is unchanged.

**Test.** New `tests/test_cookbook_default_launch_cmd.py`: (1) with a GGUF sibling the command contains `-hf <repo>` and `--hf-file <name>.gguf` and no `-hfr`; (2) with no GGUF sibling no file flag is emitted.

**How to verify.**
`python -m pytest tests/test_cookbook_default_launch_cmd.py -q`. Reverting `src/tools/cookbook.py` makes the first test fail. Manually: `llama-server -hf unsloth/Qwen3.5-4B-GGUF --hf-file Qwen3.5-4B-Q4_K_M.gguf` starts, whereas `-hfr <file>` is rejected. (Manual llama-server run not performed by us: UNVERIFIED.)

**Scope.** This PR addresses only the `-hfr` flag half of #6474. The issue's second symptom (reasoning models return empty content) is not touched; so use "Refs #6474", not "Fixes", unless the maintainer prefers otherwise.

Follows CONTRIBUTING: one fix per PR, targets `dev`.

## Commands a human runs (from `~/workspace/odysseus`)
```
git fetch origin && git checkout fix/cookbook-llamacpp-hf-file
git push fork fix/cookbook-llamacpp-hf-file   # already on the fork; only if updated
gh pr create -R odysseus-dev/odysseus --base dev --head aien-dev:fix/cookbook-llamacpp-hf-file \
  --title "fix(cookbook): pass the GGUF filename to llama-server with --hf-file (#6474)" \
  --body "Refs #6474. Problem, fix, test and verification steps as in docs/upstream/odysseus-6474-pr.md (Body section)."
```
Paste the Body section above into the PR description instead of the short `--body` if preferred.

## Other narrow Odysseus observations (not fixed)
- The 0.1 note that `tail_serve_output` is missing from `TOOL_TAGS` is STILL TRUE at 2992bf6: `'tail_serve_output' in src.agent_tools.TOOL_TAGS` is False (the cookbook block in `src/agent_tools/__init__.py` lists the other cookbook tools but not this one), while it is in the schema/tool lists (`src/tool_capabilities.py:122`, `src/tool_index.py:130`, `src/agent_loop.py:532` and `:763`, dispatched at `src/tool_execution.py:1205`). Native calls to it are rejected as unknown. Likely a one-line fix in its own PR. I found no open upstream PR for it (UNVERIFIED: only searched for 6474/hf-file).
