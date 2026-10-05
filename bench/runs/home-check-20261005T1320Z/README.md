# Smart-home maintainer check, 2026-10-05

**Not independent.** The maintainer who wrote CORE.md "Adding an adapter" (#56) and
`docs/WRITING-AN-ADAPTER.md` (#58) built this adapter. A second independent run (Gemini) was started
and stopped on the project owner's call. This checks that the new rules and guide carry a second,
different host through the corpus, including the `send_email` class the to-do run never exercised.
It is not evidence that an outsider could.

| File | What it is |
|---|---|
| `authority.py` | Smart-home host: `read_sensors` (read), `set_light` (changes home state), `send_alert` (sends out of the home), `unlock_door` (always needs a person). Written from the guide. |
| `table-entry.json` | Its `adapters.home` entry, applying the effect classes of "Adding an adapter". `delete_file` is left out (unlocking removes nothing), so delete-target cases are excluded by clause 3; the approval class maps to `unlock_door`. |
| `run_subset.py`, `runner-vs-codex.diff` | The independent run's runner (`adapter-repro-20261005T1215Z/toy/run_subset.py`) with names changed and nothing else. |
| `verdicts.json`, `verdicts-negctl.json` | Normal run and `--no-exposure-check`. |

Policy: reads authorized; `set_light` held for approval and `send_alert` denied when the exposure floor
is below `user_supplied`; `unlock_door` always held. The negative control drops the floor check.

## Result

| Line | Result |
|---|---|
| Subset (computed, `adapter_subset.py --subset home`, no runner edit needed) | 43: 30 injection, 13 approval |
| Executed / failed | 43 / 0 |
| Violations / content-derived | 0 / 0 |
| Injected intents | 32 |
| Negative control | all 30 injection cases FAIL, 32 violations |

Run on main aed1372 with the wheel built from it (`.py` stripped, as in the independent run).

## What doing it showed

- The rules carried the second host with one judgement call (delete class left out), which the new
  text names explicitly.
- `system` in a table entry has no stated meaning beyond T3/T4; `maintainer-check` was used here.
- Every adapter author still writes a runner and judge. Both runs used the same ~220-line runner,
  written once by the independent implementer from the spec text. Shipping it as a shared runner
  in `conformance/runners/` would remove the largest piece of work a newcomer has, and the place
  where a judge could quietly be wrong.

## Rerun

From a checkout of main at or after aed1372: add `table-entry.json`'s object under `adapters` in
`conformance/adapter-translation.json`, copy `authority.py` and `run_subset.py` to `adapters/home/`, then
`python3 adapters/home/run_subset.py` (exit 0) and `--no-exposure-check` (exit 1), with
`PYTHONPATH=python` or the wheel installed. Each run writes `verdicts.json` or `verdicts-negctl.json` next to
the runner; both were checked byte-identical to the files here from a fresh clone of aed1372.
