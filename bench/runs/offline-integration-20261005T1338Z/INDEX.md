# Offline integration evidence at interplane main 3fa8ee0

Commit checked: `3fa8ee007a2099e6b99ac1c20defb8137e01b9d5` (origin/main, "Shared adapter runner ... (#61)"). The checks mirror
Note: origin/main advanced to 838c666 (docs only: one new file docs/prereg/PREREG-0.2y-base-set-discovery.md, 221 lines) after the run; no code, fixture or workflow changed, so the results apply to it. This PR is based on 838c666.
`.github/workflows/ci.yml` jobs (schemas-and-fixtures, bench-structural, rust, python, package, negative-controls,
cross-language, adapter-aien, adapter-odysseus). No model, GPU or model server was used. The qualification workflow
(needs a live endpoint) was not run. Driver: `run-part1.sh` (first 14 steps) and `run.sh` (the rest), both kept as run.

Environment: rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0, Python 3.12.3 (CI pins 3.12.14; difference noted), Linux aarch64 (Spark).
Pins observed: Odysseus 2992bf6d368a11472323e47d3bfed91e79cefc6b; aegis-runtime f4e870953a466bb1cf68f1d88285929024cf1ba9;
aien-protocols 3a4cdbe8d360a2e1ddb3e8b108684048b9d4ba10; aien-sovereign-core 0bdc97a76ce5a3d198dca718f89fad9bc56deb8e (git dep in
adapters/aien/Cargo.toml and Cargo.lock); Python packages from constraints/ci-python.txt; Rust toolchain from rust-toolchain.toml.

## Result: 37 steps, 37 exit 0, 0 failed, 0 skipped, 0 not runnable
Steps from 13:38:49Z to 13:40:07Z (pip-install through py-pyflakes) overlapped a diagnostic quiet hold that began 13:39Z; rust-test,
rust-negctl-tests, rust-conformance and py-pyflakes ran during it (rust-test straddled its start). The driver was stopped at ~13:40:40Z,
the rest ran after the hold was released (~13:51Z) with a quietlock check before every step. Timing could have perturbed
nothing logical here (offline checks, no timing assertions); it is recorded only for the other session's overlap marking.
`logs/py-pytest.part1-interrupted.log` is the first, interrupted pytest run (303 passed, exit code not recorded); the recorded
run is `py-pytest` at 13:54Z.

Counts: Rust 125 passed 0 failed 0 ignored; Python 303 passed; AIEN adapter 34 passed 0 ignored; AIEN T4 32/32, violations 0;
Odysseus adapter 91 passed, no skips (CI forbids "skipped"); T3 43/43, violations 0; conformance 106 cases, Rust == Python, 0 mismatches.
The word "skip" appears in logs only in a test name (lifecycle_cannot_skip_the_decision).

## Per-step table (command is the first line of the step in steps.tsv / run.sh)
| step | exit | start UTC | end UTC | overlap | what it proves |
|---|---|---|---|---|---|
| pip-install | 0 | 2026-10-05T13:38:49Z | 2026-10-05T13:38:52Z | before hold | pinned Python test deps install from constraints/ci-python.txt |
| schema-selfcheck | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | every spec/schemas/*.json is a valid JSON Schema 2020-12 |
| fixture-validation | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | conformance fixtures validate against the schemas |
| bench-validate | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | bench corpus structure, domain rule, coverage, digests |
| bench-validate-0.2x | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:52Z | before hold | 0.2x held-out corpus structure and digests |
| live-injection-offline | 0 | 2026-10-05T13:38:52Z | 2026-10-05T13:38:53Z | before hold | live-model runner logic with a scripted endpoint (no model) |
| stats-newcombe | 0 | 2026-10-05T13:38:53Z | 2026-10-05T13:38:53Z | before hold | pre-registered statistics reproduce Newcombe 1998 Table III |
| analyze-tests | 0 | 2026-10-05T13:38:53Z | 2026-10-05T13:38:54Z | before hold | analyzer tests and synthetic-run output identical to expected |
| rust-fmt | 0 | 2026-10-05T13:38:54Z | 2026-10-05T13:38:54Z | before hold | Rust formatting |
| rust-clippy | 0 | 2026-10-05T13:38:54Z | 2026-10-05T13:38:58Z | before hold | Rust lint, warnings denied |
| rust-test | 0 | 2026-10-05T13:38:58Z | 2026-10-05T13:39:59Z | run during diagnostic hold (13:39Z+) | Rust unit and integration tests (125 passed, 0 failed) |
| rust-negctl-tests | 0 | 2026-10-05T13:39:59Z | 2026-10-05T13:40:04Z | run during diagnostic hold (13:39Z+) | negative-control feature tests detect their defects |
| rust-conformance | 0 | 2026-10-05T13:40:04Z | 2026-10-05T13:40:07Z | run during diagnostic hold (13:39Z+) | Rust verdicts for the 106 conformance cases (approval/deny/receipt/trust) |
| py-pyflakes | 0 | 2026-10-05T13:40:07Z | 2026-10-05T13:40:07Z | run during diagnostic hold (13:39Z+) | Python lint |
| py-pytest | 0 | 2026-10-05T13:54:34Z | 2026-10-05T13:55:03Z |  | Python tests (303 passed) |
| py-conformance | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | Python verdicts for the 106 conformance cases |
| cross-language | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | Rust and Python verdicts identical (106 cases, 0 mismatches) |
| trust-digest | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | trust corpus and protocol digests match conformance/TRUST-DIGEST.txt |
| negctl-not-in-pipeline | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | pipeline libraries never mention negative controls |
| default-build-refuses-variant | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:03Z |  | default build carries no variant code and exits 2 on --variant |
| negctl-matrix | 0 | 2026-10-05T13:55:03Z | 2026-10-05T13:55:06Z |  | every negative-control variant detected; Rust and Python matrices identical |
| pkg-wheel | 0 | 2026-10-05T13:55:06Z | 2026-10-05T13:55:07Z |  | Python wheel builds |
| pkg-venv-examples | 0 | 2026-10-05T13:55:07Z | 2026-10-05T13:55:09Z |  | installed wheel runs both examples from outside the checkout |
| pkg-wheel-vs-checkout | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:09Z |  | wheel conformance verdicts equal checkout verdicts |
| pkg-adapter-runner | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:09Z |  | shared adapter runner reproduces recorded verdicts and its negative control fails as recorded |
| pkg-cargo-package | 0 | 2026-10-05T13:55:09Z | 2026-10-05T13:55:20Z |  | every Rust crate packages and builds from its .crate (--locked) |
| aien-siblings | 0 | 2026-10-05T13:55:20Z | 2026-10-05T13:55:22Z |  | pinned aegis-runtime and aien-protocols checked out at PINS revs |
| aien-fmt | 0 | 2026-10-05T13:55:22Z | 2026-10-05T13:55:22Z |  | AIEN adapter formatting |
| aien-clippy | 0 | 2026-10-05T13:55:22Z | 2026-10-05T13:55:31Z |  | AIEN adapter lint |
| aien-test | 0 | 2026-10-05T13:55:31Z | 2026-10-05T13:55:42Z |  | AIEN adapter tests against real authority path (34 passed, 0 failed, 0 ignored) |
| aien-t4 | 0 | 2026-10-05T13:55:42Z | 2026-10-05T13:55:42Z |  | T4 subset through real Pipeline + AIEN authority: 32/32, violations 0 |
| odys-clone | 0 | 2026-10-05T13:55:42Z | 2026-10-05T13:55:45Z |  | Odysseus checked out at pinned 2992bf6 |
| odys-pip | 0 | 2026-10-05T13:55:45Z | 2026-10-05T13:55:59Z |  | Odysseus requirements install under constraints |
| odys-pyflakes | 0 | 2026-10-05T13:55:59Z | 2026-10-05T13:55:59Z |  | Odysseus adapter lint |
| odys-pytest | 0 | 2026-10-05T13:55:59Z | 2026-10-05T13:56:01Z |  | Odysseus adapter tests vs real Odysseus code (91 passed, no skips) |
| odys-t3 | 0 | 2026-10-05T13:56:01Z | 2026-10-05T13:56:03Z |  | T3 subset through real Pipeline + Odysseus authority: 43/43, violations 0 |
| bench-runner-offline | 0 | 2026-10-05T13:56:03Z | 2026-10-05T13:56:11Z |  | bench runner offline test with denial/approval negative controls |

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
