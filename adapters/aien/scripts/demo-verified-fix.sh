#!/usr/bin/env bash
# Reproducible demo of the verified fix-the-test workflow (VAC M4a).
#
# One command runs the fix-the-test slice gate (scripts/test-fix-the-test.sh) against a real native
# `aien-cli daemon`, records every revision it depends on, and writes $OUT/DEMO-RESULT.md. It refuses to
# pass when the daemon reports a STUB compose library, when the verdict is anything other than the
# labelled-incomplete one a scripted run may earn, or when any recorded fact disagrees with its source.
#
# Prerequisites: git, jq, make, cc, cargo/rustc, sha256sum, the Llama-3.2-1B-Instruct snapshot (see
# adapters/aien/README.md "Reproducible demo"). `quietlock check` is honoured when quietlock exists.
#
# Daemon, one of:
#   AIEN_BIN=<path>      an aien-cli built natively (omega compose library linked). Its sovereign-core rev is
#                        caller-asserted via SOVEREIGN_CORE_REV unless the daemon prints it.
#   build mode           SC_DIR (aien-sovereign-core checkout), OMEGA_DIR (HEAD == SC_DIR/omega.lock),
#                        PHYSICS_DIR (HEAD == OMEGA_DIR/physics.lock), AIENOS_REPO (HEAD == OMEGA_DIR/aienos.lock);
#                        builds `cargo build -p aien-cli` on the CPU with the same env as sovereign-core's
#                        scripts/test-linked-compose.sh. CARGO_TARGET_DIR is honoured.
# Also: OUT (fresh directory, required), AIEN_LEDGER_MODEL_DIR, SOVEREIGN_CORE_REV, OMEGA_LOCK_REV.
# No process is killed here. Exit non-zero on any failure.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
adapter="$(cd "$here/.." && pwd)"
repo="$(cd "$adapter/../.." && pwd)"
die() { echo "DEMO FAIL: $*" >&2; exit 1; }
: "${OUT:?OUT must name a fresh output directory}"
for t in git jq make cc sha256sum rustc cargo; do command -v "$t" >/dev/null || die "$t is required"; done
[ ! -e "$OUT/gate" ] || die "$OUT/gate exists: use a fresh OUT"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
if command -v quietlock >/dev/null && ! quietlock check >/dev/null 2>&1; then die "quiet flag is held (quietlock check)"; fi
model_dir="${AIEN_LEDGER_MODEL_DIR:-$HOME/.cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c}"
red() { sed -e "s#$HOME#~#g" -e "s#$repo#<interplane>#g"; }  # no home paths in the record
started="$(date -u +%FT%TZ)"
[ -z "$(git -C "$repo" status --porcelain -- adapters provenance)" ] || die "adapters/ or provenance/ has uncommitted changes: the demo records a commit, so commit or stash first"

# 1. The daemon: given, or built from the four checkouts and their locks.
build_note=""
if [ -z "${AIEN_BIN:-}" ]; then
  : "${SC_DIR:?set AIEN_BIN, or SC_DIR OMEGA_DIR PHYSICS_DIR AIENOS_REPO to build}"
  : "${OMEGA_DIR:?OMEGA_DIR}" "${PHYSICS_DIR:?PHYSICS_DIR}" "${AIENOS_REPO:?AIENOS_REPO}"
  SC_DIR="$(cd "$SC_DIR" && pwd)"; OMEGA_DIR="$(cd "$OMEGA_DIR" && pwd)"
  PHYSICS_DIR="$(cd "$PHYSICS_DIR" && pwd)"; AIENOS_REPO="$(cd "$AIENOS_REPO" && pwd)"
  [ "$(git -C "$OMEGA_DIR" rev-parse HEAD)" = "$(tr -d '[:space:]' <"$SC_DIR/omega.lock")" ] || die "omega HEAD != sovereign-core omega.lock"
  [ "$(git -C "$PHYSICS_DIR" rev-parse HEAD)" = "$(tr -d '[:space:]' <"$OMEGA_DIR/physics.lock")" ] || die "physics HEAD != omega physics.lock"
  [ "$(git -C "$AIENOS_REPO" rev-parse HEAD)" = "$(tr -d '[:space:]' <"$OMEGA_DIR/aienos.lock")" ] || die "aienos HEAD != omega aienos.lock"
  export AIEN_OMEGA_COMPOSE_DIR="$OMEGA_DIR" AIEN_PHYSICS_DIR="$PHYSICS_DIR" AIEN_AIENOS_LOCK_REPO="$AIENOS_REPO"
  unset AIEN_OMEGA_DIR AIEN_OMEGA_GPU_LIB AIEN_OMEGA_COMPOSE_LIB AIEN_OMEGA_COMPOSE_SHA AIEN_FORCE_CPU_STUB AIEN_DEV_FALLBACK
  (cd "$SC_DIR" && cargo build -p aien-cli) >"$OUT/build.log" 2>&1 || { tail -5 "$OUT/build.log" >&2; die "daemon build failed (log $OUT/build.log)"; }
  AIEN_BIN="${CARGO_TARGET_DIR:-$SC_DIR/target}/debug/aien-cli"
  SOVEREIGN_CORE_REV="$(git -C "$SC_DIR" rev-parse HEAD)"; export SOVEREIGN_CORE_REV
  [ -z "$(git -C "$SC_DIR" status --porcelain --untracked-files=no)" ] || die "sovereign-core checkout is dirty"
  build_note="built here from SC_DIR at $SOVEREIGN_CORE_REV (CPU, debug)"
else
  [ -x "$AIEN_BIN" ] || die "AIEN_BIN not executable: $AIEN_BIN"
  build_note="supplied via AIEN_BIN, not built by this run"
  OMEGA_DIR="${OMEGA_DIR:-${AIEN_OMEGA_COMPOSE_DIR:-}}"; PHYSICS_DIR="${PHYSICS_DIR:-${AIEN_PHYSICS_DIR:-}}"; AIENOS_REPO="${AIENOS_REPO:-${AIEN_AIENOS_LOCK_REPO:-}}"
  SC_DIR="${SC_DIR:-}"
fi
export AIEN_BIN
rev_of() { [ -n "$1" ] && git -C "$1" rev-parse HEAD 2>/dev/null || echo "UNVERIFIED (checkout not given)"; }
omega_rev="${OMEGA_LOCK_REV:-$(rev_of "${OMEGA_DIR:-}")}"
physics_rev="$(rev_of "${PHYSICS_DIR:-}")"; aienos_rev="$(rev_of "${AIENOS_REPO:-}")"
omega_lock="n/a"; physics_lock="n/a"; aienos_lock="n/a"
[ -n "${SC_DIR:-}" ] && [ -f "$SC_DIR/omega.lock" ] && omega_lock="$(tr -d '[:space:]' <"$SC_DIR/omega.lock")"
[ -n "${OMEGA_DIR:-}" ] && [ -f "$OMEGA_DIR/physics.lock" ] && physics_lock="$(tr -d '[:space:]' <"$OMEGA_DIR/physics.lock")"
[ -n "${OMEGA_DIR:-}" ] && [ -f "$OMEGA_DIR/aienos.lock" ] && aienos_lock="$(tr -d '[:space:]' <"$OMEGA_DIR/aienos.lock")"
bin_sha="$(sha256sum "$AIEN_BIN" | cut -d' ' -f1)"
sc_rev="${SOVEREIGN_CORE_REV:-UNVERIFIED}"
if [ -n "${SC_DIR:-}" ] && [ -d "$SC_DIR/.git" -o -f "$SC_DIR/.git" ]; then sc_rev="$(git -C "$SC_DIR" rev-parse HEAD)"; sc_src="read from SC_DIR"; else sc_src="caller-asserted (SOVEREIGN_CORE_REV), not verified by this run"; fi
export SOVEREIGN_CORE_REV="$sc_rev"
[ "$sc_rev" != "UNVERIFIED" ] || echo "note: SOVEREIGN_CORE_REV not given; recorded as UNVERIFIED" >&2

# 2. The slice gate, once. A stub daemon is refused below, from the daemon's own report.
gate_rc=0
OMEGA_LOCK_REV="$omega_rev" OUT="$OUT/gate" "$here/test-fix-the-test.sh" >"$OUT/gate.log" 2>&1 || gate_rc=$?
[ "$gate_rc" -eq 0 ] || { tail -5 "$OUT/gate.log" | red >&2; die "slice gate failed with $gate_rc (log $OUT/gate.log)"; }
g="$OUT/gate"; rec="$g/receipt.json"; b="$g/rows/slice1/bundle"; r="$b/records"
dlog="$g/rows/slice1/daemon.log"
[ -f "$rec" ] && [ -f "$b/COMPANION.json" ] && [ -f "$dlog" ] || die "gate left no receipt, bundle or daemon log"

# 3. Facts, each read from the daemon's or the run's own output.
compose_line="$(grep -m1 '^Compose:' "$dlog" || true)"
[ -n "$compose_line" ] || die "daemon log has no Compose: line"
case "$compose_line" in "Compose: native ("*) ;; *) die "STUB daemon (daemon says: $compose_line): the demo does not pass on a stub";; esac
recall_native="$(jq -r '.compose_native // "absent"' "$r/aien/compose-recall.json")"
recall_omega="$(jq -r '.omega_sha // "absent"' "$r/aien/compose-recall.json")"
[ "$recall_native" = "true" ] || die "ComposeRecall compose_native=$recall_native (need true)"
line_omega="$(printf '%s' "$compose_line" | sed -n 's/^Compose: native (omega \([0-9a-f]*\)).*/\1/p')"
[ "$line_omega" = "$recall_omega" ] || die "Compose line omega ($line_omega) != ComposeRecall omega_sha ($recall_omega)"
if [ "$omega_rev" != "${omega_rev#UNVERIFIED}" ] || [ "$omega_rev" = "n/a" ]; then omega_check="not compared (omega checkout not given)"
else [ "$omega_rev" = "$recall_omega" ] || die "omega checkout $omega_rev != daemon omega $recall_omega"; omega_check="matches the daemon's report"; fi
[ "$(jq -r .aien.native.claimed "$b/COMPANION.json")" = "true" ] || die "bundle does not claim native"
verdict="$(jq -r .verdict "$rec")"
want="PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
[ "$verdict" = "$want" ] || die "verdict is not the expected labelled-incomplete one: $verdict"
case "$verdict" in PASS|"PASS complete"*) die "a scripted run cannot be PASS complete";; esac
[ "$(jq -r .aien_cli_sha256 "$rec")" = "$bin_sha" ] || die "daemon binary hash changed during the run"

model_sha="$(sed -n 's/.*model_sha256=\([0-9a-f]*\).*/\1/p' "$dlog" | head -1)"
tok_sha="$(sed -n 's/.*tokenizer_sha256=\([0-9a-f]*\).*/\1/p' "$dlog" | head -1)"
[ -n "$model_sha" ] && [ -n "$tok_sha" ] || die "daemon load log lacks model/tokenizer sha256"
[ "$model_sha" = "$(sha256sum "$model_dir/model.safetensors" | cut -d' ' -f1)" ] || die "model sha256 in the daemon log != the file on disk"
[ "$tok_sha" = "$(sha256sum "$model_dir/tokenizer.json" | cut -d' ' -f1)" ] || die "tokenizer sha256 in the daemon log != the file on disk"
backend="$(grep -m1 -i 'backend' "$dlog" | red || true)"

grant="$(jq -r .text "$r/aien/ledger-grant.json")"; ack="$(jq -r .text "$r/aien/ledger-ack.json")"; intent="$(jq -r .text "$r/aien/ledger-intent.json")"
appr_id="$(jq -r .approval_id <<<"$grant")"; appr_key="$(jq -r .approval_key <<<"$grant")"; approver="$(jq -r .approver <<<"$grant")"
w_path="$(jq -r .path <<<"$intent")"; w_sha="$(jq -r .content_sha256 <<<"$intent")"; w_prior="$(jq -r .prior_sha256 <<<"$intent")"
ack_phase="$(jq -r .phase <<<"$ack")"; ack_disk="$(jq -r .disk_sha256 <<<"$ack")"; ack_err="$(jq -r '.disk_error // "null"' <<<"$ack")"
[ "$ack_phase" = "ack" ] && [ "$ack_disk" = "$w_sha" ] && [ "$ack_err" = "null" ] || die "ack does not confirm the approved bytes on disk"
# The intent carries no approval id; it binds to the grant by `authorization` (= the grant record id), and the
# grant carries the approval. So all three must hold: intent -> this grant, grant -> an approval, same proposal.
grant_id="$(jq -r .id "$r/aien/ledger-grant.json")"
[ "$(jq -r .authorization <<<"$intent")" = "$grant_id" ] || die "intent.authorization is not this grant ($grant_id)"
case "$appr_id" in ""|null) die "grant carries no approval_id";; esac
[ "$(jq -r .proposal_sha256 <<<"$intent")" = "$(jq -r .proposal_sha256 <<<"$grant")" ] || die "intent and grant name different proposals"
approved_content_sha="$(jq -r .content_sha256 <<<"$grant")"
[ "$approved_content_sha" = "$w_sha" ] || die "grant content sha != written content sha"
trace_content_sha="$(jq -j ".[0].payload.arguments.content" "$r/interplane/trace.json" | sha256sum | cut -d" " -f1)"
[ "$trace_content_sha" = "$w_sha" ] || die "the INTERPLANE request content digest ($trace_content_sha) != the approved content digest"
t_before_exit="$(jq -r .task.test_exit_before "$rec")"; t_after_exit="$(jq -r .task.test_exit_after "$rec")"
out_before_sha="$(jq -r .before.stdout_sha256 "$r/task/test-run.json")"; out_after_sha="$(jq -r .stdout_sha256 "$r/task/test-run.json")"
[ "$out_after_sha" = "$(sha256sum "$r/task/test-stdout.txt" | cut -d' ' -f1)" ] || die "retained stdout digest != test-run record"
neg_n="$(jq '.negative_checks | length' "$rec")"; [ "$neg_n" -eq 7 ] || die "expected 7 negative checks (control + 6), got $neg_n"
rustc_v="$(rustc --version)"
finished="$(date -u +%FT%TZ)"
mf() { jq -r "$1" "$rec"; }

# 4. DEMO-RESULT.md
res="$OUT/DEMO-RESULT.md"
{
cat <<MD
# Verified fix-the-test demo (VAC M4a)

Machine-written by \`adapters/aien/scripts/demo-verified-fix.sh\`. Started $started, finished $finished (UTC).
Labels: [O] observed in this run's own output, [I] inferred, [U] UNVERIFIED.

## 1. Revisions
| item | value |
|---|---|
| interplane HEAD | $(git -C "$repo" rev-parse HEAD) (clean adapters/ and provenance/) [O] |
| sovereign-core rev | $sc_rev ($sc_src) |
| daemon binary | $build_note |
| daemon sha256 | $bin_sha [O] |
| daemon "Compose:" line | \`$compose_line\` [O] |
| omega.lock (sovereign-core) | $omega_lock |
| omega checkout HEAD | $omega_rev ($omega_check) |
| physics.lock (omega) | $physics_lock |
| physics checkout HEAD | $physics_rev |
| aienos.lock (omega) | $aienos_lock |
| aienos checkout HEAD | $aienos_rev |
| host | $(uname -sr), $(uname -m), $(nproc) cpus [O] |
| rustc | $rustc_v [O] |
| build env | AIEN_OMEGA_COMPOSE_DIR, AIEN_PHYSICS_DIR, AIEN_AIENOS_LOCK_REPO set to the checkouts above; AIEN_OMEGA_DIR, AIEN_FORCE_CPU_STUB, AIEN_DEV_FALLBACK unset (CPU only) |
| model dir | $(printf '%s' "$model_dir" | red) |
| model.safetensors sha256 | $model_sha (daemon load log, equals file on disk) [O] |
| tokenizer.json sha256 | $tok_sha (daemon load log, equals file on disk) [O] |
| verifier binary sha256 | $(mf .verifier_sha256) |

### Native or stub, as reported by the daemon
- Daemon log: \`$compose_line\`
- ComposeRecall: compose_native=$recall_native, omega_sha=$recall_omega (retained in the bundle as \`records/aien/compose-recall.json\`) [O]
- Bundle \`aien.native\`: $(jq -c .aien.native "$b/COMPANION.json")
- The demo exits non-zero on a stub; this run was not one.

## 2. Starting task
Task id \`$(mf .task.task_id)\`. Fixture \`adapters/aien/fixtures/fix_the_test/\` (\`TASK.md\`):

\`\`\`
$(cat "$adapter/fixtures/fix_the_test/TASK.md")
\`\`\`
Source commit (harness git init of the fixture): $(mf .task.source_commit)

## 3. Authorized action
- Tool: write_file, path \`$w_path\`
- Content sha256: $w_sha (bytes the approval is bound to)
- Prior sha256 (bytes replaced): $w_prior
- Approval binding id: \`$appr_id\` (approval key $appr_key), approver \`$approver\`
- The grant, intent and ack records all carry that content digest [O]

## 4. Outcome
- Ledger ack: phase=$ack_phase, disk_sha256=$ack_disk (equals the approved content), disk_error=$ack_err [O]

## 5. Tests
| | exit code | retained stdout sha256 |
|---|---|---|
| before the write | $t_before_exit | $out_before_sha |
| after the write | $t_after_exit | $out_after_sha |

Command: \`make test\` in the task workspace (GNU make exits 2 on a failing recipe). Tree commit after: $(mf .task.tree_commit_after).

## 6. Verifier verdict
\`\`\`
$verdict
\`\`\`

## 7. Tamper summary (each on a hard-linked copy of the bundle)
\`\`\`
$(jq -r '.negative_checks[]' "$rec" | cut -c1-160)
\`\`\`

## 8. Reproduction
\`\`\`
git clone <interplane> && cd interplane
# prerequisites: git jq make cc cargo; the Llama-3.2-1B-Instruct snapshot at \$AIEN_LEDGER_MODEL_DIR
# a native aien-cli (see the build-mode comment in the script), then:
AIEN_BIN=<native aien-cli> SOVEREIGN_CORE_REV=$sc_rev OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh
# or build it: SC_DIR=<sovereign-core @ rev above> OMEGA_DIR=<omega @ omega.lock> PHYSICS_DIR=<physics @ omega's physics.lock> \\
#   AIENOS_REPO=<aienos @ omega's aienos.lock> OUT=<fresh dir> adapters/aien/scripts/demo-verified-fix.sh
\`\`\`
This run: daemon $build_note; AIEN_OMEGA_COMPOSE_DIR/AIEN_PHYSICS_DIR/AIEN_AIENOS_LOCK_REPO as in the table; the script calls \`scripts/test-fix-the-test.sh\` once (one live row, a real daemon, CPU).

## 9. What is proven
- The approved write is bound to exact bytes: grant, intent and ack share one content digest, and the ack's on-disk digest equals it [O].
- The write went through the daemon's approved ledger (claim, grant, intent, ack, committed records retained) [O].
- Tests failed before and passed after; their stdout digests are retained and checked by the verifier [O].
- The source pin matches the grant's prior_sha256; the native claim agrees with the daemon's own report; six tampers are each refused with their specific code [O].
- The daemon was native (not a stub) by its own report, and its model/tokenizer hashes equal the files on disk [O].

## 10. What is not proven
- The model turn is scripted: the verdict is labelled incomplete (\`missing=link:model_turn\`), never PASS complete. No authorship claim for the proposal.
- Records are unsigned exports: the verifier cannot tell an export from a hand-written file.
- The test run is harness evidence, not a daemon effect (the daemon only writes the file).
- The desk MAC for ComposeAuthorize stays default-off (unchanged here). No security default was changed.
- CPU reference backend only; no GPU path was exercised.
- Sovereign-core rev: $sc_src.
- The demo checks the native report after the gate's run, not before it: a stub daemon is refused (exit non-zero) but only after the run.
MD
} | red >"$res"
grep -q "$HOME" "$res" && die "home path leaked into the record"
echo "DEMO PASS (labelled incomplete): $res"
