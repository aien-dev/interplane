# Offline integration evidence at interplane main 3fa8ee0

Commit checked: `3fa8ee007a2099e6b99ac1c20defb8137e01b9d5` (origin/main, "Shared adapter runner ... (#61)"). The checks mirror
`.github/workflows/ci.yml` jobs (schemas-and-fixtures, bench-structural, rust, python, package, negative-controls,
cross-language, adapter-aien, adapter-odysseus). No model, GPU or model server was used. The qualification workflow
(needs a live endpoint) was not run. Driver: `run-part1.sh` (the first 14 recorded steps, plus a 15th, an interrupted pytest run with no recorded exit code) and `run.sh` (the rest), both kept as run.
Note: origin/main advanced to 838c666 (docs only: one new file docs/prereg/PREREG-0.2y-base-set-discovery.md, 221 lines) after the run; no code, fixture or workflow changed, so the results apply to it. This PR is based on 838c666.

Environment: rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0, Python 3.12.3 (CI pins 3.12.14; difference noted), Linux aarch64 (Spark).
Pins observed: Odysseus 2992bf6d368a11472323e47d3bfed91e79cefc6b; aegis-runtime f4e870953a466bb1cf68f1d88285929024cf1ba9;
aien-protocols 3a4cdbe8d360a2e1ddb3e8b108684048b9d4ba10; aien-sovereign-core 0bdc97a76ce5a3d198dca718f89fad9bc56deb8e (git dep in
adapters/aien/Cargo.toml and Cargo.lock); Python packages from constraints/ci-python.txt; Rust toolchain from rust-toolchain.toml.

## Result: 37 steps, 37 exit 0, 0 failed, 0 skipped, 0 not runnable
Steps from 13:38:49Z to 13:40:07Z (pip-install through py-pyflakes) overlapped a diagnostic quiet hold that ran 13:39:12Z to 13:50:23Z (history lines in OVERLAP-NOTE.txt); rust-test,
rust-negctl-tests, rust-conformance and py-pyflakes ran during it (rust-test straddled its start). The driver was stopped at ~13:40:40Z,
the rest ran after the hold was released (13:50:23Z) with a quietlock check before every step. Timing could have perturbed
nothing logical here (offline checks, no timing assertions); it is recorded only for the other session's overlap marking.
`logs/py-pytest.part1-interrupted.log` is the first, interrupted pytest run (303 passed, exit code not recorded); the recorded
run is `py-pytest` at 13:54Z.

Counts: Rust 125 passed 0 failed 0 ignored; Python 303 passed; AIEN adapter 34 passed 0 ignored; AIEN T4 32/32, violations 0;
Odysseus adapter 91 passed, no skips (CI forbids "skipped"); T3 43/43, violations 0; conformance 106 cases, Rust == Python, 0 mismatches.
The word "skip" appears in logs only in a test name (lifecycle_cannot_skip_the_decision).

## Per-step table (commands from steps.tsv, shortened with "..."; $WT is the interplane checkout, $SC and $OUT are scratch and evidence dirs; full text in run.sh)
| step | command | exit | start UTC | end UTC | overlap | what it proves |
|---|---|---|---|---|---|---|
| pip-install | `$SC/venv/bin/pip install -q -c constraints/ci-python.txt pytest jsonschema pyflakes -e python` | 0 | 2026-10-05T13:38:49Z | 2026-10-05T13:38:52Z | before hold | pinned Python test deps install from constraints/ci-python.txt |
| schema-selfcheck | `$SC/venv/bin/python - <<'PYX'` | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | every spec/schemas/*.json is a valid JSON Schema 2020-12 |
| fixture-validation | `$SC/venv/bin/python conformance/runners/validate_fixtures.py` | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | conformance fixtures validate against the schemas |
| bench-validate | `$SC/venv/bin/python bench/tools/validate.py --require-jsonschema` | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | bench corpus structure, domain rule, coverage, digests |
| bench-validate-0.2x | `$SC/venv/bin/python bench/tools/validate.py --corpus 0.2x --require-jsonschema` | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | 0.2x held-out corpus structure and digests |
| live-injection-offline | `PYTHONPATH=python PYTHONDONTWRITEBYTECODE=1 $SC/venv/bin/python bench/tools/test_live_injection.py` | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:53Z | before hold | live-model runner logic with a scripted endpoint (no model) |
| stats-newcombe | `$SC/venv/bin/python bench/tools/stats.py` | 0 | 2026-10-05T13:38:53Z | 2026-10-05T13:38:53Z | before hold | pre-registered statistics reproduce Newcombe 1998 Table III |
| analyze-tests | `$SC/venv/bin/python bench/tools/test_analyze.py && rm -rf $SC/analyze-out && $SC/venv/bin/python bench/tool...` | 0 | 2026-10-05T13:38:53Z | 2026-10-05T13:38:54Z | before hold | analyzer tests and synthetic-run output identical to expected |
| rust-fmt | `cargo fmt --all --check` | 0 | 2026-10-05T13:38:54Z | 2026-10-05T13:38:54Z | before hold | Rust formatting |
| rust-clippy | `cargo clippy --all-targets --all-features -- -D warnings` | 0 | 2026-10-05T13:38:54Z | 2026-10-05T13:38:58Z | before hold | Rust lint, warnings denied |
| rust-test | `cargo test --all` | 0 | 2026-10-05T13:38:58Z | 2026-10-05T13:39:59Z | ran during hold (13:39:12Z-13:50:23Z) | Rust unit and integration tests (125 passed, 0 failed) |
| rust-negctl-tests | `cargo test -p interplane-conformance --features negative-controls` | 0 | 2026-10-05T13:39:59Z | 2026-10-05T13:40:04Z | ran during hold (13:39:12Z-13:50:23Z) | negative-control feature tests detect their defects |
| rust-conformance | `cargo run -q -p interplane-conformance -- ../conformance/fixtures --out $OUT/rust-verdicts.json` | 0 | 2026-10-05T13:40:04Z | 2026-10-05T13:40:07Z | ran during hold (13:39:12Z-13:50:23Z) | Rust verdicts for the 106 conformance cases (approval/deny/receipt/trust) |
| py-pyflakes | `$SC/venv/bin/python -m pyflakes python/interplane python/tests` | 0 | 2026-10-05T13:40:07Z | 2026-10-05T13:40:07Z | ran during hold (13:39:12Z-13:50:23Z) | Python lint |
| py-pytest | `$SC/venv/bin/python -m pytest python/tests -q` | 0 | 2026-10-05T13:54:34Z | 2026-10-05T13:55:03Z |  | Python tests (303 passed) |
| py-conformance | `$SC/venv/bin/python -m interplane.conformance conformance/fixtures --out $OUT/py-verdicts.json` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | Python verdicts for the 106 conformance cases |
| cross-language | `$SC/venv/bin/python - <<'PYX'` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | Rust and Python verdicts identical (106 cases, 0 mismatches) |
| trust-digest | `$SC/venv/bin/python conformance/runners/trust_digest.py --check` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | trust corpus and protocol digests match conformance/TRUST-DIGEST.txt |
| negctl-not-in-pipeline | `! grep -rIl -E 'negctl\|negative-controls\|NEGCTL' rust/crates/interplane-core rust/crates/interplane-cross...` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | pipeline libraries never mention negative controls |
| default-build-refuses-variant | `cargo build -q -p interplane-conformance && ! grep -q NEGCTL_BUILT_IN target/debug/interplane-conformance &...` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | default build carries no variant code and exits 2 on --variant |
| negctl-matrix | `(cd rust && cargo run -q -p interplane-conformance --features negative-controls -- ../conformance/fixtures ...` | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:06Z |  | every negative-control variant detected; Rust and Python matrices identical |
| pkg-wheel | `rm -rf $SC/dist && $SC/venv/bin/pip wheel -q --no-deps -w $SC/dist ./python` | 0 | 2026-10-05T13:55:06Z | 2026-10-05T13:55:07Z |  | Python wheel builds |
| pkg-venv-examples | `rm -rf $SC/v && python3 -m venv $SC/v && $SC/v/bin/pip install -q $SC/dist/interplane-*.whl && cd $SC && IN...` | 0 | 2026-10-05T13:55:07Z | 2026-10-05T13:55:09Z |  | installed wheel runs both examples from outside the checkout |
| pkg-wheel-vs-checkout | `(cd $SC && $SC/v/bin/interplane-conformance $WT/conformance/fixtures --out $SC/wheel-verdicts.json) && PYTH...` | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:09Z |  | wheel conformance verdicts equal checkout verdicts |
| pkg-adapter-runner | `H=$WT/bench/runs/home-check-20261005T1320Z; rm -rf $SC/home && mkdir $SC/home && cp $H/authority.py $H/tabl...` | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:09Z |  | shared adapter runner reproduces recorded verdicts and its negative control fails as recorded |
| pkg-cargo-package | `cargo package --workspace --locked` | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:20Z |  | every Rust crate packages and builds from its .crate (--locked) |
| aien-siblings | `adapters/aien/setup-siblings.sh $CAMPAIGN` | 0 | 2026-10-05T13:55:20Z | 2026-10-05T13:55:22Z |  | pinned aegis-runtime and aien-protocols checked out at PINS revs |
| aien-fmt | `cargo fmt --all --check` | 0 | 2026-10-05T13:55:22Z | 2026-10-05T13:55:22Z |  | AIEN adapter formatting |
| aien-clippy | `cargo clippy --all-targets -- -D warnings` | 0 | 2026-10-05T13:55:22Z | 2026-10-05T13:55:31Z |  | AIEN adapter lint |
| aien-test | `cargo test` | 0 | 2026-10-05T13:55:31Z | 2026-10-05T13:55:42Z |  | AIEN adapter tests against real authority path (34 passed, 0 failed, 0 ignored) |
| aien-t4 | `T4_OUT=$OUT/t4-verdicts.json cargo test --test t4_corpus -- --nocapture` | 0 | 2026-10-05T13:55:42Z | 2026-10-05T13:55:42Z |  | T4 subset through real Pipeline + AIEN authority: 32/32, violations 0 |
| odys-clone | `[ -d $CAMPAIGN/odysseus-pin/.git ] \|\| git clone -q https://github.com/odysseus-dev/odysseus.git $CAMPAIGN...` | 0 | 2026-10-05T13:55:42Z | 2026-10-05T13:55:45Z |  | Odysseus checked out at pinned 2992bf6 |
| odys-pip | `$SC/venv/bin/pip install -q -c constraints/ci-python.txt -r $CAMPAIGN/odysseus-pin/requirements.txt pytest ...` | 0 | 2026-10-05T13:55:45Z | 2026-10-05T13:55:59Z |  | Odysseus requirements install under constraints |
| odys-pyflakes | `$SC/venv/bin/python -m pyflakes adapters/odysseus` | 0 | 2026-10-05T13:55:59Z | 2026-10-05T13:55:59Z |  | Odysseus adapter lint |
| odys-pytest | `ODYSSEUS_SRC=$CAMPAIGN/odysseus-pin PYTHONPATH=$WT/python:. PYTHONDONTWRITEBYTECODE=1 $SC/venv/bin/python -...` | 0 | 2026-10-05T13:55:59Z | 2026-10-05T13:56:01Z |  | Odysseus adapter tests vs real Odysseus code (91 passed, no skips) |
| odys-t3 | `ODYSSEUS_SRC=$CAMPAIGN/odysseus-pin PYTHONPATH=$WT/python:adapters/odysseus PYTHONDONTWRITEBYTECODE=1 $SC/v...` | 0 | 2026-10-05T13:56:01Z | 2026-10-05T13:56:03Z |  | T3 subset through real Pipeline + Odysseus authority: 43/43, violations 0 |
| bench-runner-offline | `ODYSSEUS_SRC=$CAMPAIGN/odysseus-pin PYTHONDONTWRITEBYTECODE=1 $SC/venv/bin/python bench/tools/test_runner_o...` | 0 | 2026-10-05T13:56:03Z | 2026-10-05T13:56:11Z |  | bench runner offline test with denial/approval negative controls |

## Comparison with earlier results
- REPORT-0.1 (offline): Rust 6 crates 84 tests, Python 175 tests, 23 conformance cases, Odysseus 55 tests, AIEN 10 pipeline tests, aien-sovereign-core 7580039.
  Now: 125 Rust, 303 Python, 106 cases, 91 Odysseus tests, 34 AIEN tests, sovereign-core 0bdc97a. All pass; the suite is much larger, same checks and more.
- REPORT-0.3 / trust-0.3-20261005T0229Z (offline gates, commit 0c69bb5): Odysseus, aegis-runtime and aien-protocols pins are identical to now.
  aien-sovereign-core was 0c1d249, now 0bdc97a (changed pin). T4 verdicts here are byte-identical to that run's t4-verdicts.json;
  T3 verdicts differ only in two metadata lines (odysseus_head abbreviation and source path). Same rustc 1.99.0 and Python 3.12.3.
- The live-model reports (REPORT-0.2 qual-20261004T2207Z at manifest commit 661ea66, REPORT-0.3 live legs) used earlier revisions
  and sovereign-core pins; this run does not re-run any live-model result and says nothing about model behavior.
- Observation, not a failure: running `cargo` in adapters/aien rewrote the tracked Cargo.lock (interplane-adapter-aien version
  0.3.0 -> 0.1.0, i.e. the committed lock disagrees with Cargo.toml). Saved as cargo-lock-drift.patch; I reverted the working copy.
  CI does not use --locked for that crate, so CI passes too.
- Not covered: the manual qualification workflow, the optional Rust-vs-Python cargo-deny style checks (none exist in CI), and any live model.

Hold evidence: see OVERLAP-NOTE.txt (quiet-flag history lines). Steps starting after 13:50:23Z ran outside the hold.
