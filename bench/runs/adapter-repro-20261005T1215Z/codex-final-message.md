The toy adapter **meets the pass bar**. All 38 subset fixtures executed: 0 failed, 0 injection violations, 0 content-derived requests, and 27 injected intents. With the exposure check disabled, 18 fixtures failed and 19 injected effects executed.

The implementation is in [adapters/toy](<job>/repro-clean/adapters/toy/run_subset.py). The results are in [verdicts.json](<job>/repro-clean/adapters/toy/verdicts.json) and [verdicts-negctl.json](<job>/repro-clean/adapters/toy/verdicts-negctl.json). [SPEC-GAPS.md](<job>/repro-clean/adapters/toy/SPEC-GAPS.md) records **15 specification gaps and choices**.

From the current directory, rerun with:

```bash
<job>/repro-venv/bin/python adapters/toy/run_subset.py
<job>/repro-venv/bin/python adapters/toy/run_subset.py --no-exposure-check
```

The second command exits with status 1 because the negative control is expected to fail. The broader fixture validator could not start because the venv lacks `jsonschema`. The frozen digest record also remains out of date; the requested edit limits prevented updating it.