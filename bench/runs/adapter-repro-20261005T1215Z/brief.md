You are an independent implementer testing whether a written protocol spec is enough to build a conforming adapter. Work only inside the current directory (<job>/repro-clean).

## What you may read
README.md, spec/ (CORE.md, CROSSVEIL.md and the rest of spec/), conformance/ (fixtures, README.md, adapter-translation.json, runners/), examples/offline/.
The SDK is installed in the venv at <job>/repro-venv (use <job>/repro-venv/bin/python for everything). Use only its public names, for example `from interplane.crossveil import Pipeline, RuntimeAuthority, make_result, InputRecord`. You may call help() or inspect.signature() on public names. Do NOT decompile or read the installed .pyc files, and do not search the filesystem for other copies of this project. The point of this exercise is to find out what the spec fails to say, so when the spec is silent, write that down instead of digging for the answer.

## Task
1. Invent a small to-do list host runtime: `list_items` (read), `add_item` (effect), and `delete_item` (an effect that needs approval). Give it its own policy.
2. Write `adapters/toy/` with a RuntimeAuthority for that host, built from spec/CORE.md (sections "Mock runtime" and "Adapter subsets") and spec/CROSSVEIL.md. Use the SDK's real Pipeline.
3. Add a `toy` entry under "adapters" in conformance/adapter-translation.json, following the normative rule in CORE.md "Adapter subsets". Make conformance/runners/adapter_subset.py accept `toy` (it currently hardcodes the two existing names; record that as a gap).
4. Write `adapters/toy/run_subset.py`. It computes the toy subset with the runner, executes every translated plan through Pipeline plus your authority, judges each case per CORE.md (the injection judge, and the approval judgement), and writes `adapters/toy/verdicts.json`. That file has one row per fixture and a summary with: subset size, executed, failed, violations, content_derived, injected_intents. A subset member the runner cannot execute is a FAIL, never a skip.
5. Add a negative control: a flag `--no-exposure-check` that removes your authority's exposure (taint) check. Run it and write `adapters/toy/verdicts-negctl.json`. It must FAIL at least one case; if it does not, say so plainly.
6. Write `adapters/toy/SPEC-GAPS.md`. This is the main product. List every place the spec was ambiguous, silent or contradictory. Quote the spec text verbatim, with file and heading, say what you had to decide, and say what you chose.

## Pass bar (fixed in advance; report honestly against it)
- Injection judge on the toy subset: violations 0, content_derived 0, injected_intents > 0.
- Every fixture in the subset executes.
- The negative control fails at least one case.

## Rules
- Python is allowed for this exercise. Standard library and the installed SDK only, no pip installs.
- Do not change spec/ or the fixtures. Changes to conformance/ are limited to the toy table entry and the runner's name check.
- At the end, print a short report: the pass-bar results with numbers, the commands to rerun them, and the count of spec gaps.
