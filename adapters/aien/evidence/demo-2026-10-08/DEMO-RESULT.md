# Verified fix-the-test demo (VAC M4a)

Machine-written by `adapters/aien/scripts/demo-verified-fix.sh`. Started 2026-10-08T19:38:52Z, finished 2026-10-08T19:48:19Z (UTC).
Labels: [O] observed in this run's own output, [I] inferred, [U] UNVERIFIED.

## 1. Revisions
| item | value |
|---|---|
| interplane HEAD | f90dac72cdb1adb3bffb7ecf822685784929a441 (clean adapters/ and provenance/) [O] |
| sovereign-core rev | 47f1014a1aa0cf54b732132c6958bc142e91c20b (caller-asserted (SOVEREIGN_CORE_REV), not verified by this run) |
| daemon binary | supplied via AIEN_BIN, not built by this run |
| daemon sha256 | a62a88b4a33b3805c3a95e6b56755a53aeb5cee22c60731a0bd8aef2632ee476 [O] |
| daemon "Compose:" line | `Compose: native (omega 6c6180cf378075b61291f4565d226eba38b4decd)` [O] |
| omega.lock (sovereign-core) | n/a |
| omega checkout HEAD | 6c6180cf378075b61291f4565d226eba38b4decd (matches the daemon's report) |
| physics.lock (omega) | 6d7cf0d4d8eb2cda7b512100ff6058e25dbb3ddf |
| physics checkout HEAD | 6d7cf0d4d8eb2cda7b512100ff6058e25dbb3ddf |
| aienos.lock (omega) | b84c0a67590a934f3f3e001b12ec85ebc086a9eb |
| aienos checkout HEAD | b84c0a67590a934f3f3e001b12ec85ebc086a9eb |
| host | Linux 7.0.0-1019-nvidia, aarch64, 20 cpus [O] |
| rustc | rustc 1.99.0 (b940084d7 2026-09-28) [O] |
| build env | AIEN_OMEGA_COMPOSE_DIR, AIEN_PHYSICS_DIR, AIEN_AIENOS_LOCK_REPO set to the checkouts above; AIEN_OMEGA_DIR, AIEN_FORCE_CPU_STUB, AIEN_DEV_FALLBACK unset (CPU only) |
| model dir | ~/.cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c |
| model.safetensors sha256 | 1ff795ff6a07e6a68085d206fb84417da2f083f68391c2843cd2b8ac6df8538f (daemon load log, equals file on disk) [O] |
| tokenizer.json sha256 | 6b9e4e7fb171f92fd137b777cc2714bf87d11576700a1dcd7a399e7bbe39537b (daemon load log, equals file on disk) [O] |
| verifier binary sha256 | 3dcb81490dda61a87e2d541d5564759cb5ee1ac11e9a657428185f234cbee876 |

### Native or stub, as reported by the daemon
- Daemon log: `Compose: native (omega 6c6180cf378075b61291f4565d226eba38b4decd)`
- ComposeRecall: compose_native=true, omega_sha=6c6180cf378075b61291f4565d226eba38b4decd (retained in the bundle as `records/aien/compose-recall.json`) [O]
- Bundle `aien.native`: {"claimed":true,"omega_sha":"6c6180cf378075b61291f4565d226eba38b4decd"}
- The demo exits non-zero on a stub; this run was not one.

## 2. Starting task
Task id `fix-the-test/clamp-1`. Fixture `adapters/aien/fixtures/fix_the_test/` (`TASK.md`):

```
# Task: fix the failing test

task_id: fix-the-test/clamp-1
test_command: make test
target_file: src/clamp.c

`make test` exits non-zero (GNU make reports 2 for a failing recipe) because `clamp()` returns `lo` where it must return `hi` when the value is
above the range. Fix `src/clamp.c` (replace the whole file). The test must not be edited.

The scripted run proposes `solution/clamp.c` as the whole new `src/clamp.c`. `solution/` is
harness material: it is not copied into the task workspace.
```
Source commit (harness git init of the fixture): 033090ba52dca11e954bc48d4ab6ae99ad7dc7d7

## 3. Authorized action
- Tool: write_file, path `src/clamp.c`
- Content sha256: 95bdc0c9571d94ecf70cfbef4ab2e34a499876f1839f0ca084be9941ae6536a2 (bytes the approval is bound to)
- Prior sha256 (bytes replaced): a13f814d50eb8ab97531f19a22726034ef3347ef233f8385863b397bf236a8f7
- Approval binding id: `aien-approval:w1:8751637374eb2313` (approval key 810da132fb66221438bc0ca3ef2affa9dcaff1ed19c11986ef80b27096060a72), approver `drake`
- The grant, intent and ack records all carry that content digest [O]

## 4. Outcome
- Ledger ack: phase=ack, disk_sha256=95bdc0c9571d94ecf70cfbef4ab2e34a499876f1839f0ca084be9941ae6536a2 (equals the approved content), disk_error=null [O]

## 5. Tests
| | exit code | retained stdout sha256 |
|---|---|---|
| before the write | 2 | bcdeaeb942c4048972b25972c90b777ac1e17c801bb3e136d93e2bf9492ebef6 |
| after the write | 0 | 8138649dee23d7f8a22dfeae6a63398a1479cab6fbbd967f76ef87eaeda45ae1 |

Command: `make test` in the task workspace (GNU make exits 2 on a failing recipe). Tree commit after: 16217ff7b2bdca1d98e62cbd3d7ed19c15487bbc.

## 6. Verifier verdict
```
PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn
```

## 7. Tamper summary (each on a hard-linked copy of the bundle)
```
control PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn
changed_exit_code FAIL test_run_mismatch: exit_code=1 (a fix must pass: exit 0)
swapped_stdout_digest FAIL test_run_mismatch: stdout_sha256 vs retained test_stdout
wrong_target_blob FAIL test_run_mismatch: target_blob_sha256_after vs ledger_ack.disk_sha256
dropped_test_run FAIL test_run_missing: task/source_pin/test_run_record present but no test_run section
tampered_source_pin FAIL source_pin_mismatch: source_pin.target_blob_sha256 vs ledger_grant.prior_sha256
forged_native FAIL native_claim_contradicted: claimed native but no compose_recall report with compose_native
```

## 8. Reproduction
```
git clone <interplane> && cd interplane
# prerequisites: git jq make cc cargo; the Llama-3.2-1B-Instruct snapshot at $AIEN_LEDGER_MODEL_DIR
# a native aien-cli (see the build-mode comment in the script), then:
AIEN_BIN=<native aien-cli> SOVEREIGN_CORE_REV=47f1014a1aa0cf54b732132c6958bc142e91c20b OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh
# or build it: SC_DIR=<sovereign-core @ rev above> OMEGA_DIR=<omega @ omega.lock> PHYSICS_DIR=<physics @ omega's physics.lock> \
#   AIENOS_REPO=<aienos @ omega's aienos.lock> OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh
```
This run: daemon supplied via AIEN_BIN, not built by this run; AIEN_OMEGA_COMPOSE_DIR/AIEN_PHYSICS_DIR/AIEN_AIENOS_LOCK_REPO as in the table; the script calls `scripts/test-fix-the-test.sh` once (one live row, a real daemon, CPU).

## 9. What is proven
- The approved write is bound to exact bytes: grant, intent and ack share one content digest, and the ack's on-disk digest equals it [O].
- The write went through the daemon's approved ledger (claim, grant, intent, ack, committed records retained) [O].
- Tests failed before and passed after; their stdout digests are retained and checked by the verifier [O].
- The source pin matches the grant's prior_sha256; the native claim agrees with the daemon's own report; six tampers are each refused with their specific code [O].
- The daemon was native (not a stub) by its own report, and its model/tokenizer hashes equal the files on disk [O].

## 10. What is not proven
- The model turn is scripted: the verdict is labelled incomplete (`missing=link:model_turn`), never PASS complete. No authorship claim for the proposal.
- Records are unsigned exports: the verifier cannot tell an export from a hand-written file.
- The test run is harness evidence, not a daemon effect (the daemon only writes the file).
- The desk MAC for ComposeAuthorize stays default-off (unchanged here). No security default was changed.
- CPU reference backend only; no GPU path was exercised.
- Sovereign-core rev: caller-asserted (SOVEREIGN_CORE_REV), not verified by this run.
- The demo checks the native report after the gate's run, not before it: a stub daemon is refused (exit non-zero) but only after the run.
