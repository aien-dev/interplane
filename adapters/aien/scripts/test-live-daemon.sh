#!/usr/bin/env bash
# Live verified-run gate for the INTERPLANE AIEN adapter.
#
# Runs the adapter's live-daemon tests (they are #[ignore]d: they start a real `aien-cli daemon`,
# ~6.5 min each on the CPU reference backend), then feeds the bundle row 1 exported to the offline
# provenance verifier and checks the verdict, then tampers with copies of the bundle and requires
# the verifier to refuse each with a named code. It cannot pass vacuously: zero tests run, a
# missing required row, a failed test, a missing bundle or an unexpected verdict all fail it.
#
# Inputs (environment):
#   AIEN_BIN               native aien-cli binary (required)
#   AIEN_LEDGER_MODEL_DIR  dir with model.safetensors, tokenizer.json, config.json
#                          (default: the unsloth Llama-3.2-1B-Instruct snapshot in the HF cache)
#   OUT                    output dir (required; rows/ live.log receipt.json are written here)
# Optional:
#   LIVE_ROWS              space-separated test-name filters (default: every live test). Row 1 and
#                          the four attack tests (x1_ x2_ x3_ x4_) are ALWAYS required to have run.
#   SOVEREIGN_CORE_REV     revision the binary was built from; the daemon does not print it, so it is
#                          recorded as caller-asserted (default UNVERIFIED)
#   OMEGA_LOCK_REV         omega revision of the linked compose library; default: git rev-parse of
#                          $AIEN_OMEGA_COMPOSE_DIR when set, else UNVERIFIED
#   CARGO_TARGET_DIR       build dir
# Exit 0 only when every check passed. No process is killed here; the daemons are the tests' own.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
adapter="$(cd "$here/.." && pwd)"
repo="$(cd "$adapter/../.." && pwd)"
die() { echo "GATE FAIL: $*" >&2; exit 1; }

: "${AIEN_BIN:?AIEN_BIN must name the native aien-cli binary}"
: "${OUT:?OUT must name the output directory}"
[ -x "$AIEN_BIN" ] || die "AIEN_BIN not executable: $AIEN_BIN"
command -v jq >/dev/null || die "jq is required"
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
required="row1_ x1_ x2_ x3_ x4_"
rows="${LIVE_ROWS:-}"
if [ -n "$rows" ]; then
  for r in $required; do
    case " $rows " in *" $r "*) ;; *) die "LIVE_ROWS must include $r (required rows: $required)";; esac
  done
fi

# 1. The live tests.
log="$OUT/live.log"
set +e
(cd "$adapter" && AIEN_BIN="$AIEN_BIN" AIEN_LEDGER_MODEL_DIR="$model_dir" LEDGER_OUT="$OUT/rows" \
  cargo test --locked --test compose_ledger --test compose_ledger_attacks -- --ignored --test-threads=1 $rows) >"$log" 2>&1
test_rc=$?
set -e
[ "$test_rc" -eq 0 ] || { tail -5 "$log" >&2; die "live tests exited $test_rc (log $log)"; }
passed="$(grep -c '^test .* \.\.\. ok$' "$log" || true)"
failed="$(grep -c '^test .* \.\.\. FAILED$' "$log" || true)"
[ "$passed" -ge 1 ] || die "zero live tests ran (vacuous pass refused)"
[ "$failed" -eq 0 ] || die "$failed live tests failed"
for r in $required; do
  grep -q "^test .*${r}.* \.\.\. ok$" "$log" || die "required test $r did not run and pass"
done
tests_json="$(grep '^test .* \.\.\. ok$' "$log" | sed 's/^test \(.*\) \.\.\. ok$/\1/' | jq -R . | jq -s .)"

# 2. The verifier on the freshly exported row-1 bundle.
bundle="$OUT/rows/row1/bundle"
[ -f "$bundle/COMPANION.json" ] || die "row 1 exported no bundle: $bundle/COMPANION.json"
verify() { (cd "$repo/provenance" && cargo run -q --locked --release -- verify "$1" 2>&1 || true) | tail -1; }
want_incomplete="PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
verdict="$(verify "$bundle")"
case "$verdict" in
  "$want_incomplete") ;;
  "PASS complete "*) ;;
  *) die "unexpected verdict on the live bundle: $verdict";;
esac

# 3. Tamper checks on hard-linked copies (a changed file is written fresh, never in place).
neg="$OUT/negative"
mkdir -p "$neg"
fork() { rm -rf "$neg/$1"; cp -al "$bundle" "$neg/$1"; }
restamp() { # bundle record-name: set the manifest's sha256 and size to the file as it is now
  local d="$1" n="$2" p; p="$(jq -r ".records.$n.path" "$d/COMPANION.json")"
  jq --arg n "$n" --arg s "$(sha256sum "$d/$p" | cut -d' ' -f1)" --argjson b "$(wc -c <"$d/$p")" \
    '.records[$n].sha256=$s | .records[$n].bytes=$b' "$d/COMPANION.json" >"$d/COMPANION.tmp"
  mv "$d/COMPANION.tmp" "$d/COMPANION.json"
}
rewrite() { # bundle record-name sed-expression: same-length byte edit, written as a fresh file
  local d="$1" n="$2" p; p="$(jq -r ".records.$n.path" "$d/COMPANION.json")"
  sed "$3" "$d/$p" >"$d/$p.new"
  cmp -s "$d/$p" "$d/$p.new" && die "tamper edit changed nothing in $p"
  [ "$(wc -c <"$d/$p")" -eq "$(wc -c <"$d/$p.new")" ] || die "tamper edit changed the size of $p"
  rm "$d/$p"; mv "$d/$p.new" "$d/$p"
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
# (a) the proposal content in the trace changes; the manifest still names the old digest.
fork content_bytes
rewrite "$neg/content_bytes" interplane_trace 's/Friday/Monday/'
expect content_bytes "$neg/content_bytes" digest_mismatch
# (b) same change, manifest restamped to hide it: the grant's content digest still disagrees.
fork content_restamped
rewrite "$neg/content_restamped" interplane_trace 's/Friday/Monday/'
restamp "$neg/content_restamped" interplane_trace
expect content_restamped "$neg/content_restamped" binding_mismatch
# (c) the ack record's state changes (same-length byte edit); the manifest still names the old digest.
fork ack_bytes
rewrite "$neg/ack_bytes" ledger_ack 's/\\"state\\":\\"DONE\\"/\\"state\\":\\"HOLD\\"/'
expect ack_bytes "$neg/ack_bytes" digest_mismatch
# (d) same change, restamped: the daemon-written ack must say DONE.
fork ack_restamped
rewrite "$neg/ack_restamped" ledger_ack 's/\\"state\\":\\"DONE\\"/\\"state\\":\\"HOLD\\"/'
restamp "$neg/ack_restamped" ledger_ack
expect ack_restamped "$neg/ack_restamped" binding_mismatch
# (e) a retained file is truncated (partial write).
fork truncated
p="$(jq -r .records.ledger_intent.path "$neg/truncated/COMPANION.json")"
head -c 20 "$neg/truncated/$p" >"$neg/truncated/$p.new"; rm "$neg/truncated/$p"; mv "$neg/truncated/$p.new" "$neg/truncated/$p"
expect truncated "$neg/truncated" size_mismatch

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
  --arg sc_rev "${SOVEREIGN_CORE_REV:-UNVERIFIED}" \
  --arg sc_rev_basis "$([ -n "${SOVEREIGN_CORE_REV:-}" ] && echo 'caller-asserted: the daemon does not print its revision' || echo 'the daemon does not print its revision')" \
  --arg omega "${omega:-UNVERIFIED}" --arg verdict "$verdict" \
  --arg companion_sha "$(sha256sum "$bundle/COMPANION.json" | cut -d' ' -f1)" \
  --argjson tests "$tests_json" --argjson passed "$passed" \
  --arg started "$started" --arg finished "$finished" \
  --arg negatives "$(cat "$neg/verdicts.txt")" \
  '{kind: "interplane-aien-live-gate-receipt", adapter_commit: $adapter_commit, adapter_dirty: $adapter_dirty,
    sovereign_core_bin_sha256: $bin_sha, sovereign_core_rev: $sc_rev, sovereign_core_rev_basis: $sc_rev_basis,
    omega_lock: $omega, backend: "CPU reference (as printed in the daemon log)",
    verdict: $verdict, bundle_companion_sha256: $companion_sha,
    tests: {passed: $passed, names: $tests}, negative_checks: ($negatives | split("\n") | map(select(length>0))),
    started: $started, finished: $finished,
    limits: ["records are unsigned exports; the verifier cannot tell an export from a hand-written file",
             "desk MAC for ComposeAuthorize stays default-off (not changed here)",
             "native compose library linking is not reported by the daemon",
             "the model turn is scripted: proposal authorship is not claimed"]}' >"$OUT/receipt.json"
echo "GATE PASS: $passed live tests, verdict: $verdict; receipt $OUT/receipt.json"
