# Independent adapter reproduction, 2026-10-05

Plan 19.7 item 8, second half. Report: `docs/REPORT-adapter-repro.md`.

## What is here

| File | What it is |
|---|---|
| `brief.md` | The full brief the implementer received (job paths replaced by `<job>`). |
| `toy/authority.py`, `toy/run_subset.py` | The implementer's adapter and runner, unedited apart from path replacement. |
| `toy/verdicts.json`, `toy/verdicts-negctl.json` | Its verdicts: normal, and with `--no-exposure-check`. |
| `toy/SPEC-GAPS.md` | The 15 gaps it found, quoted verbatim from the spec. |
| `conformance.diff` | Its only edits outside `adapters/toy/`: the `toy` table entry and the runner's name check. |
| `codex-final-message.md` | Its closing report. |

## Setup

- Implementer: Codex CLI 0.160.0, model `gpt-6-sol`, `codex exec -s workspace-write`, about 106k tokens, under 30 minutes of wall time.
- Source: tag `v0.3.0` (d33ef18). The implementer's directory held only `README.md`, `LICENSE`, `spec/`,
  `conformance/`, `examples/` and an empty `adapters/`, with no `.git`.
- SDK: `pip install ./python` from a separate v0.3.0 clone into a fresh venv, then every installed `.py`
  replaced by its compiled `.pyc` (`compileall -b`) so no SDK source was readable. The offline example passed
  against that install. The implementer was told not to decompile; its log shows no attempt (the only SDK
  paths in it are traceback lines from errors it hit).

## Rerun

From a v0.3.0 checkout with the wheel installed in `<venv>`:

```
git apply bench/runs/adapter-repro-20261005T1215Z/conformance.diff
mkdir -p adapters/toy && cp bench/runs/adapter-repro-20261005T1215Z/toy/*.py adapters/toy/
<venv>/bin/python adapters/toy/run_subset.py                       # exit 0
<venv>/bin/python adapters/toy/run_subset.py --no-exposure-check   # exit 1 (expected)
```

Checked 2026-10-05 in a fresh v0.3.0 clone with these exact steps: both verdict files byte-identical, exit codes 0 and 1.
