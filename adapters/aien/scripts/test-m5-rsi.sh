#!/usr/bin/env bash
# VAC M5 gate: bounded RSI, independently judged, through the approved path.
#
# Runs the live row `m5_rsi_judged_change_lands_and_rolls_back` against a real `aien-cli daemon`:
# the fixture's tests fail, the RSI engine (as RSI_USER) proposes a README change, the separate
# judge (as JUDGE_USER, own key) builds and evaluates parent and candidate on holdouts and signs a
# version 2 receipt, the harness refuses to propose unless the receipt binds the exact bytes, the
# change lands through ComposeApprovedProposal with the normal approval, the tests pass, and a
# second approved write rolls it back. The exported bundle is then checked by the offline
# verifier with the operator's pinned judge key, and each counterfeit must be refused with its
# own code. It cannot pass vacuously: a missing row, bundle, receipt or verdict fails it.
#
# Inputs (environment): AIEN_BIN, OUT (fresh), SPARK_RSI_DIR (release spark-rsi and
# spark-rsi-judge), M5_JUDGE_KEY (the operator's pinned judge public key file, made once by
# m5-setup-judge.sh); AIEN_LEDGER_MODEL_DIR, SOVEREIGN_CORE_REV, SPARK_RSI_REV as in the slice gate.
# No process is killed here; the daemon is the test's own.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
adapter="$(cd "$here/.." && pwd)"
repo="$(cd "$adapter/../.." && pwd)"
die() { echo "M5 GATE FAIL: $*" >&2; exit 1; }

: "${AIEN_BIN:?AIEN_BIN must name the aien-cli binary}"
: "${OUT:?OUT must name the output directory}"
: "${SPARK_RSI_DIR:?SPARK_RSI_DIR must hold release spark-rsi and spark-rsi-judge}"
: "${M5_JUDGE_KEY:?M5_JUDGE_KEY must name the pinned judge public key file}"
for b in "$AIEN_BIN" "$SPARK_RSI_DIR/spark-rsi" "$SPARK_RSI_DIR/spark-rsi-judge"; do
  [ -x "$b" ] || die "not executable: $b"
done
[ -s "$M5_JUDGE_KEY" ] || die "no pinned judge key at $M5_JUDGE_KEY (run m5-setup-judge.sh once)"
command -v jq >/dev/null || die "jq is required"
model_dir="${AIEN_LEDGER_MODEL_DIR:-$HOME/.cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c}"
if command -v quietlock >/dev/null && ! quietlock check >/dev/null 2>&1; then
  die "quiet flag is held (quietlock check): no long run now"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
[ ! -e "$OUT/rows" ] || die "$OUT/rows exists: use a fresh OUT (a sealed run is never overwritten)"
started="$(date -u +%FT%TZ)"
export SPARK_RSI_DIR M5_JUDGE_KEY
export M5_POLICY="$adapter/m5/policy.json" M5_HOLDOUTS="$adapter/m5/holdouts"
export M5_HOOK="$here/m5-propose-and-judge.sh"

# 0. The pinned policy pins this holdout set.
pinned="$(jq -r .holdout_set_sha256 "$M5_POLICY")"
actual="$("$SPARK_RSI_DIR/spark-rsi-judge" --holdouts-dir "$M5_HOLDOUTS" --print-holdout-digest)"
[ "$pinned" = "$actual" ] || die "policy pins holdout set $pinned but $M5_HOLDOUTS is $actual"

# 1. The live row.
log="$OUT/live.log"
set +e
(cd "$adapter" && AIEN_BIN="$AIEN_BIN" AIEN_LEDGER_MODEL_DIR="$model_dir" LEDGER_OUT="$OUT/rows" \
  cargo test --locked --test compose_ledger -- --ignored --test-threads=1 m5_) >"$log" 2>&1
rc=$?
set -e
[ "$rc" -eq 0 ] || { tail -5 "$log" >&2; die "live row exited $rc (log $log)"; }
grep -q '^test m5_rsi_judged_change_lands_and_rolls_back \.\.\. ok$' "$log" || die "the M5 row did not run and pass"
[ "$(grep -c '^test .* \.\.\. FAILED$' "$log" || true)" -eq 0 ] || die "a live test failed"
row="$OUT/rows/m5"
bundle="$row/bundle"
[ -f "$bundle/COMPANION.json" ] || die "the row exported no bundle"
[ "$(jq -r .evaluation.binding "$bundle/COMPANION.json")" = "rsi-eval/2" ] || die "bundle has no rsi-eval/2 section"

# 2. The verifier, with the operator's pinned key and without it.
(cd "$repo/provenance" && cargo build -q --locked --release) || die "verifier build failed"
vbin="${CARGO_TARGET_DIR:-$repo/provenance/target}/release/interplane-provenance"
# The operator pins the judge key and the policy; neither is ever read from the bundle.
policy_pin="$(sha256sum "$M5_POLICY" | cut -d' ' -f1)"
verify() { "$vbin" verify "$1" --judge-key "${2:-$M5_JUDGE_KEY}" --policy-sha256 "$policy_pin" 2>&1 | tail -1 || true; }
strong="effect=aien-ledger-slice/1:strong proposal=scripted_turn"
case "$(jq -r '.aien.native.claimed // "absent"' "$bundle/COMPANION.json")" in
  true) want="PASS_LABELLED_INCOMPLETE missing=link:model_turn $strong";;
  *) want="PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native $strong";;
esac
verdict="$(verify "$bundle")"
[ "$verdict" = "$want" ] || die "unexpected verdict: $verdict (wanted: $want)"
nokey="$("$vbin" verify "$bundle" 2>&1 | tail -1 || true)"
case "$nokey" in "FAIL evaluation_untrusted:"*) ;; *) die "without the judge key the bundle must be untrusted, got: $nokey";; esac
jq -e '.verdict == "PASS" and .test_exit_after == 0 and .test_exit_before != 0
  and .rollback.disk_sha256 == .target_blob_sha256_before and .rollback.receipt.state == "DONE"' \
  "$row/receipt.json" >/dev/null || die "row receipt does not show green-after, red-before and a restoring rollback"

# 3. Counterfeits. Bundle edits go to hard-linked copies (a changed file is written fresh).
neg="$OUT/negative"; mkdir -p "$neg"; : >"$neg/verdicts.txt"
# The negatives judge from a clean clone of the task workspace at the commit the row was judged
# from (the row leaves its workspace rolled back, which is an uncommitted change).
task_commit="$(jq -r .parent_id "$row/judged/receipt.json")"
git clone -q "$row/ws" "$neg/ws" && git -C "$neg/ws" checkout -q --detach "$task_commit" \
  || die "cannot check out the task commit $task_commit for the negatives"
fork() { rm -rf "$neg/$1"; cp -al "$bundle" "$neg/$1"; }
manifest() { local d="$1" f="$2"; shift 2
  jq "$@" "$f" "$d/COMPANION.json" >"$d/COMPANION.tmp" && mv "$d/COMPANION.tmp" "$d/COMPANION.json"; }
recpath() { jq -r ".records.$2.path" "$1/COMPANION.json"; }
restamp() { local d="$1" n="$2" p; p="$(recpath "$d" "$n")"
  manifest "$d" '.records[$n].sha256=$s | .records[$n].bytes=$b' --arg n "$n" \
    --arg s "$(sha256sum "$d/$p" | cut -d' ' -f1)" --argjson b "$(wc -c <"$d/$p")"; }
editrec() { local d="$1" n="$2" f="$3" p; shift 3; p="$(recpath "$d" "$n")"
  jq "$@" "$f" "$d/$p" >"$d/$p.new"; rm "$d/$p"; mv "$d/$p.new" "$d/$p"; restamp "$d" "$n"; }
swaprec() { local d="$1" n="$2" src="$3" p; p="$(recpath "$d" "$n")"
  rm "$d/$p"; cp "$src" "$d/$p"; restamp "$d" "$n"; }
expect() { # name bundle code [key]
  local v; v="$(verify "$2" "${4:-}")"
  case "$v" in "FAIL $3:"*) echo "negative $1: $v";; *) die "negative $1: wanted FAIL $3, got: $v";; esac
  echo "$1 $v" >>"$neg/verdicts.txt"; }
refused() { # name log pattern: the hook (and so the judge) refused before any receipt
  grep -q "$3" "$2" || die "negative $1: wanted a refusal matching '$3' (log $2)"
  echo "negative $1: refused ($3)"; echo "$1 REFUSED $3" >>"$neg/verdicts.txt"; }
hook() { # name [VAR=value ...]: run the hook on the row's workspace into a fresh dir
  local name="$1"; shift; local o="$neg/$name.judged"; rm -rf "$o"; mkdir -p "$o"
  env M5_GATE_NEGATIVE=1 "$@" "$M5_HOOK" "$neg/ws" "$o" >"$o.log" 2>&1; }

fork control; [ "$(verify "$neg/control")" = "$verdict" ] || die "negative control: an untouched copy no longer verifies"
echo "control $verdict" >>"$neg/verdicts.txt"

# Substituted score: the receipt's numbers edited after signing.
fork score;  editrec "$neg/score" evaluation_receipt '.binding.holdouts_passed = 3'; expect substituted_score_count "$neg/score" evaluation_signature_bad
fork layer;  editrec "$neg/layer" evaluation_receipt '.layer_results[0].score = 0.25'; expect substituted_score_layer "$neg/layer" evaluation_signature_bad
fork subj;   editrec "$neg/subj" evaluation_receipt '.binding.subject_sha256 = "9999999999999999999999999999999999999999999999999999999999999999"'; expect edited_subject "$neg/subj" evaluation_signature_bad
# Altered policy: the retained policy edited.
fork pol;    editrec "$neg/pol" evaluation_policy '.min_holdout_pass_ratio = 0.5'; expect altered_policy_record "$neg/pol" evaluation_policy_mismatch
# Wrong key: the verifier is given a different judge key.
tk="$neg/throwaway.key"; (umask 077; head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$tk")
"$SPARK_RSI_DIR/spark-rsi-judge" --signing-key-file "$tk" --public-key >"$neg/throwaway.pub"
fork wrongkey; expect wrong_pinned_key "$neg/wrongkey" evaluation_signature_bad "$neg/throwaway.pub"
fork dropped; manifest "$neg/dropped" 'del(.evaluation)'; expect dropped_evaluation "$neg/dropped" evaluation_missing

# Real counterfeits from the judge itself. Substituted change: the real judge, real key, a
# different (also dash-free) README: a valid signature over the wrong bytes.
printf '# rsi-dashes\n\nA different README the judge also passes.\n' >"$neg/other-readme.md"
hook other M5_SUBJECT_OVERRIDE="$neg/other-readme.md" || die "judge run for the substituted change failed ($neg/other.judged.log)"
fork change; swaprec "$neg/change" evaluation_receipt "$neg/other.judged/receipt.json"; expect substituted_change "$neg/change" evaluation_subject_mismatch
# A receipt signed by a key that is not the pinned judge key (what a proposer could make).
hook forged M5_JUDGE_KEY_OVERRIDE="$tk" || die "judge run with the throwaway key failed ($neg/forged.judged.log)"
fork forged; swaprec "$neg/forged" evaluation_receipt "$neg/forged.judged/receipt.json"; expect receipt_signed_by_other_key "$neg/forged" evaluation_signature_bad
# A version 1 receipt from the real judge: verifiable history, never enough to promote.
hook v1 M5_RECEIPT_V1=1 || die "judge run for a version 1 receipt failed ($neg/v1.judged.log)"
fork v1; swaprec "$neg/v1" evaluation_receipt "$neg/v1.judged/receipt.json"; expect version_1_receipt "$neg/v1" evaluation_v1_insufficient
# The judge evaluated under a different policy (same holdouts, lower bar): its signed policy
# digest no longer matches the retained policy.
jq '.min_holdout_pass_ratio = 0.5' "$M5_POLICY" >"$neg/lower-bar-policy.json"
hook lowbar M5_POLICY="$neg/lower-bar-policy.json" || die "judge run under the lower-bar policy failed ($neg/lowbar.judged.log)"
fork lowbar; swaprec "$neg/lowbar" evaluation_receipt "$neg/lowbar.judged/receipt.json"; expect judged_under_other_policy "$neg/lowbar" evaluation_policy_mismatch
# The same receipt with its own policy alongside: self-consistent, but not the operator's policy.
fork lowpair; swaprec "$neg/lowpair" evaluation_receipt "$neg/lowbar.judged/receipt.json"
swaprec "$neg/lowpair" evaluation_policy "$neg/lowbar.judged/policy.json"; expect unpinned_policy_pair "$neg/lowpair" evaluation_policy_mismatch

# Incomplete or corrupted holdouts: the judge refuses to evaluate at all.
mkh() { rm -rf "$neg/$1"; cp -r "$M5_HOLDOUTS" "$neg/$1"; }
mkh h_missing; rm "$neg/h_missing/HOLD-M5-001.json"
! hook h_missing M5_HOLDOUTS="$neg/h_missing" || die "judge evaluated with no holdout files"
refused missing_holdouts "$neg/h_missing.judged.log" "No valid holdout suites"
mkh h_corrupt; printf '{"suite_id":' >"$neg/h_corrupt/HOLD-M5-001.json"
! hook h_corrupt M5_HOLDOUTS="$neg/h_corrupt" || die "judge evaluated a corrupted holdout file"
refused corrupted_holdouts "$neg/h_corrupt.judged.log" "is corrupt"
mkh h_shrunk; jq '.cases |= .[1:]' "$M5_HOLDOUTS/HOLD-M5-001.json" >"$neg/h_shrunk/HOLD-M5-001.json"
! hook h_shrunk M5_HOLDOUTS="$neg/h_shrunk" || die "judge evaluated a shrunken holdout set"
refused shrunken_holdouts "$neg/h_shrunk.judged.log" "does not match the policy"
mkh h_extra; echo note >"$neg/h_extra/README.txt"
! hook h_extra M5_HOLDOUTS="$neg/h_extra" || die "judge evaluated a holdout dir with a stray file"
refused stray_holdout_file "$neg/h_extra.judged.log" "unexpected holdout entry"

# RSI aimed at a file the task does not allow: a workspace whose only dashes are in CONTRIBUTING.md.
odd="$neg/ws-contributing"; rm -rf "$odd"; mkdir -p "$odd"
tar -C "$row/ws" --exclude=./.git --exclude=./target -cf - . | tar -C "$odd" -xf -
printf '# rsi-dashes\n\nNo dashes here.\n' >"$odd/README.md"
printf 'Contributions welcome\xe2\x80\x94small ones.\n' >"$odd/CONTRIBUTING.md"
git -C "$odd" init -q && git -C "$odd" add -A && git -C "$odd" -c user.name=m5 -c user.email=m5@localhost commit -qm odd
o="$neg/other_target.judged"; rm -rf "$o"; mkdir -p "$o"
! "$M5_HOOK" "$odd" "$o" >"$o.log" 2>&1 || die "a proposal for CONTRIBUTING.md reached the judge"
refused rsi_targets_other_file "$o.log" "only README.md is allowed: refused"
# A workspace that is not its own commit: the judged parent would be mislabelled.
dirty="$neg/ws-dirty"; rm -rf "$dirty"; cp -a "$neg/ws" "$dirty"; echo "extra" >>"$dirty/TASK.md"
o="$neg/dirty_workspace.judged"; rm -rf "$o"; mkdir -p "$o"
! "$M5_HOOK" "$dirty" "$o" >"$o.log" 2>&1 || die "a workspace with uncommitted changes reached the judge"
refused dirty_workspace "$o.log" "uncommitted changes"
# A workspace with no git history of its own: its parent commit cannot be named.
bare="$neg/ws-nogit"; rm -rf "$bare"; mkdir -p "$bare"
tar -C "$neg/ws" --exclude=./.git -cf - . | tar -C "$bare" -xf -
o="$neg/no_git_workspace.judged"; rm -rf "$o"; mkdir -p "$o"
! "$M5_HOOK" "$bare" "$o" >"$o.log" 2>&1 || die "a workspace without its own git repository reached the judge"
refused no_git_workspace "$o.log" "not the top of its own git repository"

# 4. Receipt.
finished="$(date -u +%FT%TZ)"
dirty=false; [ -z "$(git -C "$repo" status --porcelain -- adapters provenance 2>/dev/null)" ] || dirty=true
jq -n \
  --arg adapter_commit "$(git -C "$repo" rev-parse HEAD)" --argjson adapter_dirty "$dirty" \
  --arg bin_sha "$(sha256sum "$AIEN_BIN" | cut -d' ' -f1)" \
  --arg verifier_sha "$(sha256sum "$vbin" | cut -d' ' -f1)" \
  --arg judge_sha "$(sha256sum "$SPARK_RSI_DIR/spark-rsi-judge" | cut -d' ' -f1)" \
  --arg rsi_sha "$(sha256sum "$SPARK_RSI_DIR/spark-rsi" | cut -d' ' -f1)" \
  --arg judge_pub "$(cat "$M5_JUDGE_KEY")" --arg policy_pin "$policy_pin" \
  --arg sc_rev "${SOVEREIGN_CORE_REV:-UNVERIFIED}" --arg rsi_rev "${SPARK_RSI_REV:-UNVERIFIED}" \
  --arg verdict "$verdict" --arg nokey "$nokey" \
  --argjson run "$(jq '{task_id, proposal_id, source_commit, tree_commit_after, test_exit_before, test_exit_after, target_blob_sha256_before, target_blob_sha256_after, evaluation_receipt_sha256, evaluation_policy_sha256, rollback: {disk_sha256: .rollback.disk_sha256, test_exit_after_rollback: .rollback.test_exit_after_rollback}}' "$row/receipt.json")" \
  --argjson binding "$(jq '.binding + {admitted, format_version}' "$bundle/records/evaluation/receipt.json")" \
  --arg companion_sha "$(sha256sum "$bundle/COMPANION.json" | cut -d' ' -f1)" \
  --arg negatives "$(cat "$neg/verdicts.txt")" --arg started "$started" --arg finished "$finished" \
  '{kind: "interplane-aien-m5-rsi-receipt", adapter_commit: $adapter_commit, adapter_dirty: $adapter_dirty,
    aien_cli_sha256: $bin_sha, verifier_sha256: $verifier_sha, spark_rsi_rev: $rsi_rev,
    spark_rsi_sha256: $rsi_sha, spark_rsi_judge_sha256: $judge_sha, judge_public_key: $judge_pub, operator_policy_sha256: $policy_pin,
    sovereign_core_rev: $sc_rev, backend: "CPU reference (as printed in the daemon log)",
    task: $run, evaluation: $binding, verdict: $verdict, verdict_without_judge_key: $nokey,
    bundle_companion_sha256: $companion_sha,
    negative_checks: ($negatives | split("\n") | map(select(length>0))),
    started: $started, finished: $finished,
    limits: ["the INTERPLANE model turn is scripted by the harness from the RSI proposal: no model authored it",
             "the RSI proposer is a deterministic style scan (spark-rsi propose), not a learning system",
             "the operator account has passwordless sudo and can read the judge key: separation holds against the proposing account, not against the operator",
             "the holdout cases are in this public repository: their integrity is pinned by digest, their secrecy is not claimed",
             "the policy sets the speed allowance (non_inferiority_margin_pct) to 200%: the fixture runs in microseconds and identical programs measured up to 79% apart (speed-noise.txt), so the speed check only catches gross slowdowns here",
             "the test run is harness evidence; records are unsigned exports apart from the judge receipt",
             "desk MAC for ComposeAuthorize stays default-off (not changed here)"]}' >"$OUT/receipt.json"
echo "M5 GATE PASS: verdict: $verdict; receipt $OUT/receipt.json"
