# Reproduction of run qual-20261004T2207Z

Checked 2026-10-04 on a clean checkout of `origin/main` at `a5a4549` (branch `0.2/evidence-reproduce`), host
Ubuntu 24.04 aarch64, Python 3.12.3, cargo/rustc 1.98.1. No model, GPU or Ollama call was made. Nothing under
`bench/runs/qual-20261004T2207Z/` was changed (git shows only the new `-analysis` directory).
The historical verdict (gate FAIL on O1 and O2) is untouched.

## 1. Analyzer re-run: identical

```
python3 bench/tools/analyze.py bench/runs/qual-20261004T2207Z --out "$TMP/reanalysis"
cmp bench/runs/qual-20261004T2207Z/{summary.json,summary.md,tasks.csv} "$TMP/reanalysis/<same name>"
```

`summary.json`, `summary.md` and `tasks.csv` are byte-identical to the committed files. Differences: none.

## 2. Integrity: all checks pass

| check | result |
|---|---|
| receipts | 74 files = 37 task ids x {A, B}; every id has both; set equals manifest `task_ids` |
| qual/dev split | the 37 tasks with `split: qual` equal the manifest ids; the 7 `dev` tasks are the ones PROTOCOL/README name (filesystem-001, ambiguous-001, multidomain-001, rare-001, exec_failure-001, approval-001, injection_workspace-001) and none was run |
| corpus digests | `validate.py` recomputes `tasks_digest`, `inputs_digest`, `protocol_sha256`: equal to `CORPUS-DIGEST.txt` and to `manifest.corpus.frozen` |
| prompt | sha256 of `bench/prompts/system.md` equals `system_prompt_digest` (`207dd449...`) |
| catalog | `catalog_digest` recomputed by `load_catalog()` = `4ec0d10e...`, 71 capabilities (this is the canonical digest, not the file's sha256) |
| code | sha256 of `run_bench.py` and `bench_eval.py` equal the manifest `runner` pins; `analyze.py`, tasks, prompts, fixtures, stubs, `adapters/odysseus` and `python/interplane` have no diff between the manifest's commit `661ea66` and main (only one new test file under `python/tests`) |
| probe report | file sha256 equals `probe_report.file_sha256` |
| deterministic identity | rebuilt with the committed `build_identity()` (task, fixture, stub, fault, round-1 selection and tool digests for all 37 tasks): `identity_digest` `d01ec02d...` identical, whole object equal |
| receipts vs manifest | every receipt's `task.digest` equals the manifest per-task digest; all 161 condition-A rounds carry the manifest A `tools_digest` |
| token units | all 74 receipts declare `tokens_model_reported` (backend `usage.prompt_tokens`), as PROTOCOL section 6 and the summary state |
| pipeline infra | `infra_error` is null in all 74 receipts |

The identity check used a scratch script (not committed) that imports `bench/tools/run_bench.py` and calls
`make_cx` and `build_identity` with Odysseus at `2992bf6`. It does not contact the model endpoint.

## 3. Gates on the clean checkout

| gate (from `.github/workflows/ci.yml`) | result |
|---|---|
| schema self-check (`spec/schemas`, 10 files) | pass |
| `conformance/runners/validate_fixtures.py` | pass (25 cases, 5 envelopes, 31 dialect fixtures) |
| `bench/tools/validate.py --require-jsonschema` | pass, 44 tasks |
| `bench/tools/stats.py` (Newcombe 1998 Table III) | pass |
| `bench/tools/test_analyze.py` + synthetic run diff | pass; `diff -r` identical |
| rust: `cargo fmt --all --check`, `clippy --all-targets --all-features -D warnings`, `cargo test --all` | pass; 92 tests, 0 failed |
| rust conformance | 0 of 25 cases failed |
| python: pyflakes, `pytest python/tests` | pass; 192 passed |
| python conformance | 25/25 |
| cross-language | Rust and Python verdict JSON: 25 cases, 0 mismatches (same comparison as the CI job) |
| adapter-aien: fmt, clippy `-D warnings`, `cargo test` | pass; 20 tests (6 + 14), 0 failed |
| adapter-odysseus: pyflakes, pytest with real Odysseus `2992bf6` | pass; 55 passed, 0 skipped (CI requires no skips) |

Commands were the workflow's own, with these local differences: a throwaway venv instead of the runner's Python;
`CARGO_TARGET_DIR` and `--out` paths in a scratch directory; `-p no:cacheprovider`.

## 4. Non-reproducible or unrecorded fields

- Model outputs, wall-clock and latency (`wall_ms`, `model_ms`, `timings`), `concurrent_load` and GPU samples are
  measurements of the 2026-10-04 run. Re-analysis reproduces the statistics from them; only a new model run would
  produce new values, and the manifest records another process (`llama-server:34495`) using the GPU during the run.
  Not re-run here (rule: no model calls).
- Receipts keep argument keys and digests, not argument values, so call arguments cannot be re-read.
- The Python version that produced the original analysis is not recorded; 3.12.3 reproduces it byte for byte.
- Manifest `interplane.commit` `661ea66` is not an ancestor of main (the PR was squash-merged as `2bad9ac`). The
  content it names is identical for the files listed above, but the commit id alone cannot be checked out from main.
- Manifest `odysseus.src` is a local scratch path (`.../jobs/f3c074e9/tmp/odysseus-2992bf6`), not portable. The
  commit `2992bf6d...` is recorded and the adapter tests load it.

## 5. Dependency pinning findings

Pinned:
- Rust workspace: `rust/Cargo.lock` and `adapters/aien/Cargo.lock` exist, with registry versions locked.
- AIEN adapter git dependencies: `aien-capability` and `aien-mcp` at `aien-sovereign-core` rev `6554aac7`, locked in
  `Cargo.lock`.
- Odysseus: CI and runner require commit `2992bf6` (runner refuses another commit).
- Sibling checkouts: `aegis-runtime` `f4e87095...` and `aien-protocols` `3a4cdbe8...` are pinned in `ci.yml`.

Sibling-checkout requirement (`adapters/aien`): `aegis` is a path dependency on
`../../../interplane-audit/aegis-runtime`, and aegis-runtime reaches `../aien-protocols/crates/*`. The repository must
sit next to a directory `interplane-audit/` holding both checkouts at the pinned commits. Here
`~/workspace/interplane-audit/{aegis-runtime,aien-protocols}` were at exactly those commits (aegis-runtime has one
untracked file, `mojo/libaegis_simd.so`, unused by the build). The pin lives only in `ci.yml` and the README, not in
any lock file; nothing in the build checks the commit.

Not pinned:
- No `rust-toolchain.toml`; CI uses `dtolnay/rust-toolchain@stable`. Local result used 1.98.1.
- Python: `pip install jsonschema` / `pytest jsonschema pyflakes` with no versions, no lock file. Local versions:
  jsonschema 4.26.0, pytest 9.1.1, pyflakes 4.0.2. Odysseus `requirements.txt` has no version pins (fastapi,
  pydantic, SQLAlchemy and others float). Local: fastapi 0.142.2, pydantic 2.13.5, SQLAlchemy 2.1.3,
  numpy 2.5.3, mcp 1.30.0.
- GitHub Actions are pinned by tag (`checkout@v4`, `setup-python@v5`, `upload-artifact@v4`), not by commit.
- License text differs between crates: root `LICENSE` is Apache-2.0 and `python/pyproject.toml` says Apache-2.0, while
  `adapters/aien/Cargo.toml` says AGPL-3.0-or-later (noted only; outside this lane).
