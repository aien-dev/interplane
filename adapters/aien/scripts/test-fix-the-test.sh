#!/usr/bin/env bash
# Fix-the-test slice gate (VAC M3b) for the INTERPLANE AIEN adapter.
#
# Runs the live row `slice1_fix_the_test_lands_and_tests_pass` against a real `aien-cli daemon`
# (~6.5 min on the CPU reference backend): the fixture's `make test` fails, a SCRIPTED proposal
# carrying the corrected file goes through the daemon's approved ledger, `make test` passes. The
# exported bundle is then judged by the offline provenance verifier and tampered copies must each
# be refused with their specific code. It cannot pass vacuously: a missing row, a failed test, a
# missing bundle or an unexpected verdict fails it.
#
# Expected verdict for a scripted run (never `PASS complete`, the model turn is scripted):
#   daemon reports compose_native (sovereign-core #346 or later):
#     PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn
#   daemon without the field (older build): the same with missing=link:model_turn,link:native
#
# Inputs (environment): AIEN_BIN (required), OUT (required, fresh), AIEN_LEDGER_MODEL_DIR (default:
# the unsloth Llama-3.2-1B-Instruct snapshot), SOVEREIGN_CORE_REV (caller-asserted), OMEGA_LOCK_REV.
# No process is killed here; the daemon is the test's own.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
adapter="$(cd "$here/.." && pwd)"
repo="$(cd "$adapter/../.." && pwd)"
die() { echo "SLICE GATE FAIL: $*" >&2; exit 1; }

: "${AIEN_BIN:?AIEN_BIN must name the aien-cli binary}"
: "${OUT:?OUT must name the output directory}"
[ -x "$AIEN_BIN" ] || die "AIEN_BIN not executable: $AIEN_BIN"
command -v jq >/dev/null || die "jq is required"
command -v make >/dev/null || die "make is required"
model_dir="${AIEN_LEDGER_MODEL_DIR:-$HOME/.cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c}"
for f in model.safetensors tokenizer.json config.json; do
  [ -f "$model_dir/$f" ] || die "model dir lacks $f: $model_dir"
done
if command -v quietlock >/dev/null && ! quietlock check >/dev/null 2>&1; then
  die "quiet flag is held (quietlock check): no long run now"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
[ ! -e "$OUT/rows" ] || die "$OUT/rows exists: use a fresh OUT (a sealed run is never overwritten)"
started="$(date -u +%FT%TZ)"

# 1. The live row.
log="$OUT/live.log"
set +e
(cd "$adapter" && AIEN_BIN="$AIEN_BIN" AIEN_LEDGER_MODEL_DIR="$model_dir" LEDGER_OUT="$OUT/rows" \
  cargo test --locked --test compose_ledger -- --ignored --test-threads=1 slice1_) >"$log" 2>&1
rc=$?
set -e
[ "$rc" -eq 0 ] || { tail -5 "$log" >&2; die "live row exited $rc (log $log)"; }
grep -q '^test slice1_fix_the_test_lands_and_tests_pass \.\.\. ok$' "$log" || die "the slice row did not run and pass"
[ "$(grep -c '^test .* \.\.\. FAILED$' "$log" || true)" -eq 0 ] || die "a live test failed"

# 2. The verifier on the exported bundle.
bundle="$OUT/rows/slice1/bundle"
[ -f "$bundle/COMPANION.json" ] || die "the row exported no bundle: $bundle/COMPANION.json"
verify() { (cd "$repo/provenance" && cargo run -q --locked --release -- verify "$1" 2>&1 || true) | tail -1; }
strong="effect=aien-ledger-slice/1:strong proposal=scripted_turn"
native_claimed="$(jq -r '.aien.native.claimed // "absent"' "$bundle/COMPANION.json")"
case "$native_claimed" in
  true) want="PASS_LABELLED_INCOMPLETE missing=link:model_turn $strong"; native_label="proven: daemon reports compose_native=true and the verifier checked the retained report";;
  false) want="PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native $strong"; native_label="link:native missing: the daemon reports a STUB compose library";;
  absent) want="PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native $strong"; native_label="link:native missing: this daemon build does not report compose_native";;
  *) die "unreadable aien.native.claimed: $native_claimed";;
esac
verdict="$(verify "$bundle")"
[ "$verdict" = "$want" ] || die "unexpected verdict on the slice bundle: $verdict (wanted: $want)"
before_exit="$(jq -r .test_exit_before "$OUT/rows/slice1/receipt.json")"
after_exit="$(jq -r .test_exit_after "$OUT/rows/slice1/receipt.json")"
[ "$before_exit" != "0" ] && [ "$before_exit" != "null" ] || die "test exit before the fix was $before_exit (must be non-zero)"
[ "$after_exit" = "0" ] || die "test exit after the fix was $after_exit (must be 0)"

# 3. Tamper checks on hard-linked copies (a changed file is always written fresh, never in place).
neg="$OUT/negative"
mkdir -p "$neg"
fork() { rm -rf "$neg/$1"; cp -al "$bundle" "$neg/$1"; }
manifest() { # bundle jq-filter [jq args...]: rewrite COMPANION.json
  local d="$1" f="$2"; shift 2
  jq "$@" "$f" "$d/COMPANION.json" >"$d/COMPANION.tmp" && mv "$d/COMPANION.tmp" "$d/COMPANION.json"
}
recpath() { jq -r ".records.$2.path" "$1/COMPANION.json"; }
restamp() { # bundle record: set the manifest's sha256 and size to the file as it is now
  local d="$1" n="$2" p; p="$(recpath "$d" "$n")"
  manifest "$d" '.records[$n].sha256=$s | .records[$n].bytes=$b' --arg n "$n" \
    --arg s "$(sha256sum "$d/$p" | cut -d' ' -f1)" --argjson b "$(wc -c <"$d/$p")"
}
editrec() { # bundle record jq-filter [jq args...]: edit a JSON record as a fresh file, restamp
  local d="$1" n="$2" f="$3" p; shift 3; p="$(recpath "$d" "$n")"
  jq "$@" "$f" "$d/$p" >"$d/$p.new"; rm "$d/$p"; mv "$d/$p.new" "$d/$p"
  restamp "$d" "$n"
}
expect() { # name bundle code
  local v; v="$(verify "$2")"
  case "$v" in "FAIL $3:"*) echo "negative $1: $v";; *) die "negative $1: wanted FAIL $3, got: $v";; esac
  echo "$1 $v" >>"$neg/verdicts.txt"
}
: >"$neg/verdicts.txt"
fork control
[ "$(verify "$neg/control")" = "$verdict" ] || die "negative control: an untouched copy no longer verifies"
echo "control $verdict" >>"$neg/verdicts.txt"
other="9999999999999999999999999999999999999999999999999999999999999999"
fork exit_code;    editrec "$neg/exit_code" test_run_record '.exit_code = 1';                       expect changed_exit_code "$neg/exit_code" test_run_mismatch
fork stdout_digest; editrec "$neg/stdout_digest" test_run_record '.stdout_sha256 = $o' --arg o "$other"; expect swapped_stdout_digest "$neg/stdout_digest" test_run_mismatch
fork blob_after;   editrec "$neg/blob_after" test_run_record '.target_blob_sha256_after = $o' --arg o "$other"; expect wrong_target_blob "$neg/blob_after" test_run_mismatch
fork dropped;      manifest "$neg/dropped" 'del(.test_run)';                                         expect dropped_test_run "$neg/dropped" test_run_missing
fork pin;          editrec "$neg/pin" source_pin '.target_blob_sha256 = $o' --arg o "$other";       expect tampered_source_pin "$neg/pin" source_pin_mismatch
fork forged_native
if [ "$native_claimed" = "true" ]; then
  # a native claim whose evidence (the retained ComposeRecall report) is removed
  manifest "$neg/forged_native" 'del(.records.compose_recall)'
else
  # native:true asserted over a stub or absent recall report, with the missing-label edited to match
  manifest "$neg/forged_native" '.aien.native = {claimed: true, omega_sha: "6c6180cf378075b61291f4565d226eba38b4decd"} | .completeness.missing -= ["link:native"]'
fi
expect forged_native "$neg/forged_native" native_claim_contradicted

# 4. Receipt.
finished="$(date -u +%FT%TZ)"
omega="${OMEGA_LOCK_REV:-}"
if [ -z "$omega" ] && [ -n "${AIEN_OMEGA_COMPOSE_DIR:-}" ]; then
  omega="$(git -C "$AIEN_OMEGA_COMPOSE_DIR" rev-parse HEAD 2>/dev/null || true)"
fi
dirty=false; [ -z "$(git -C "$repo" status --porcelain -- adapters provenance 2>/dev/null)" ] || dirty=true
jq -n \
  --arg adapter_commit "$(git -C "$repo" rev-parse HEAD)" --argjson adapter_dirty "$dirty" \
  --arg bin_sha "$(sha256sum "$AIEN_BIN" | cut -d' ' -f1)" \
  --arg verifier_sha "$(sha256sum "${CARGO_TARGET_DIR:-$repo/provenance/target}/release/interplane-provenance" 2>/dev/null | cut -d' ' -f1)" \
  --arg sc_rev "${SOVEREIGN_CORE_REV:-UNVERIFIED}" --arg omega "${omega:-UNVERIFIED}" \
  --arg verdict "$verdict" --arg native_label "$native_label" \
  --argjson run "$(jq '{task_id, source_commit, tree_commit_after, test_exit_before, test_exit_after, target_blob_sha256_before, target_blob_sha256_after}' "$OUT/rows/slice1/receipt.json")" \
  --argjson native "$(jq '.aien.native // null' "$bundle/COMPANION.json")" \
  --arg companion_sha "$(sha256sum "$bundle/COMPANION.json" | cut -d' ' -f1)" \
  --arg negatives "$(cat "$neg/verdicts.txt")" --arg started "$started" --arg finished "$finished" \
  '{kind: "interplane-aien-fix-the-test-receipt", adapter_commit: $adapter_commit, adapter_dirty: $adapter_dirty,
    aien_cli_sha256: $bin_sha, verifier_sha256: $verifier_sha, sovereign_core_rev: $sc_rev,
    omega_lock: $omega, backend: "CPU reference (as printed in the daemon log)",
    task: $run, native: $native, native_label: $native_label,
    verdict: $verdict, bundle_companion_sha256: $companion_sha,
    negative_checks: ($negatives | split("\n") | map(select(length>0))),
    started: $started, finished: $finished,
    limits: ["the model turn is scripted: proposal authorship is not claimed",
             "the test run is harness evidence, not a daemon effect; the daemon only wrote the file",
             "records are unsigned exports; the verifier cannot tell an export from a hand-written file",
             "desk MAC for ComposeAuthorize stays default-off (not changed here)",
             "make exits 2 (not 1) on a failing recipe: the gate requires non-zero before and 0 after"]}' >"$OUT/receipt.json"
echo "SLICE GATE PASS: verdict: $verdict; receipt $OUT/receipt.json"
