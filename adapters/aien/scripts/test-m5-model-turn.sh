#!/usr/bin/env bash
# VAC M5 with a REAL model turn: the gate for the row `m5_model_turn_proposal_is_judged_lands_and_chains`
# plus the two restart rows (row12, row13).
#
# The loaded model (daemon StreamTurn, CPU) writes the tool call that proposes the README change; the
# separate judge evaluates the exact bytes; the change lands through ComposeApprovedProposal; the
# bundle is checked by the offline verifier with the operator's pinned judge key and policy. The
# verdict must be `PASS complete ... proposal=model_generation/2` (or, when the daemon does not
# report a native compose library, the same with `missing=link:native`), and each counterfeit of the
# model turn must be refused or labelled. It cannot pass vacuously: a missing row, bundle or verdict
# fails it. No process is killed except the test's own daemon inside the restart rows.
#
# Inputs (environment): AIEN_BIN, OUT (fresh), SPARK_RSI_DIR (release spark-rsi-judge), M5_JUDGE_KEY
# (the operator's pinned judge public key file), AIEN_LEDGER_MODEL_DIR, SOVEREIGN_CORE_REV.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
adapter="$(cd "$here/.." && pwd)"
repo="$(cd "$adapter/../.." && pwd)"
die() { echo "M5 MODEL-TURN GATE FAIL: $*" >&2; exit 1; }
: "${AIEN_BIN:?AIEN_BIN must name the aien-cli binary}"
: "${OUT:?OUT must name the output directory}"
: "${SPARK_RSI_DIR:?SPARK_RSI_DIR must hold release spark-rsi and spark-rsi-judge}"
: "${M5_JUDGE_KEY:?M5_JUDGE_KEY must name the pinned judge public key file}"
for b in "$AIEN_BIN" "$SPARK_RSI_DIR/spark-rsi" "$SPARK_RSI_DIR/spark-rsi-judge"; do
  [ -x "$b" ] || die "not executable: $b"
done
[ -s "$M5_JUDGE_KEY" ] || die "no pinned judge key at $M5_JUDGE_KEY"
command -v jq >/dev/null || die "jq is required"
model_dir="${AIEN_LEDGER_MODEL_DIR:-$HOME/.cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c}"
if command -v quietlock >/dev/null && ! quietlock check >/dev/null 2>&1; then
  die "quiet flag is held (quietlock check): no long run now"
fi
# CPU only: a GPU backend would change what the evidence means.
unset AIEN_GPU_BACKEND AIEN_REQUIRE_BLACKWELL AIEN_OMEGA_DIR
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
[ ! -e "$OUT/rows" ] || die "$OUT/rows exists: use a fresh OUT"
started="$(date -u +%FT%TZ)"
export SPARK_RSI_DIR M5_JUDGE_KEY
export M5_POLICY="$adapter/m5/policy.json" M5_HOLDOUTS="$adapter/m5/holdouts" M5_HOOK="$here/m5-propose-and-judge.sh"
pinned="$(jq -r .holdout_set_sha256 "$M5_POLICY")"
actual="$("$SPARK_RSI_DIR/spark-rsi-judge" --holdouts-dir "$M5_HOLDOUTS" --print-holdout-digest)"
[ "$pinned" = "$actual" ] || die "policy pins holdout set $pinned but $M5_HOLDOUTS is $actual"

# 1. The live rows.
log="$OUT/live.log"
set +e
(cd "$adapter" && AIEN_BIN="$AIEN_BIN" AIEN_LEDGER_MODEL_DIR="$model_dir" LEDGER_OUT="$OUT/rows" \
  cargo test --locked --test compose_ledger -- --ignored --test-threads=1 \
  m5_model_turn row12_daemon_restart row13_restart) >"$log" 2>&1
rc=$?
set -e
[ "$rc" -eq 0 ] || { tail -8 "$log" >&2; die "live rows exited $rc (log $log)"; }
for t in m5_model_turn_proposal_is_judged_lands_and_chains row12_daemon_restart_does_not_make_a_spent_approval_spendable \
         row13_restart_between_intent_and_ack_is_unresolved_not_success; do
  grep -q "^test $t \.\.\. ok$" "$log" || die "row $t did not run and pass"
done
row="$OUT/rows/m5mt"; bundle="$row/bundle"
[ -f "$bundle/COMPANION.json" ] || die "the row exported no bundle"
jq -e '.effect.proposal_origin == "model_generation/2"' "$bundle/COMPANION.json" >/dev/null || die "bundle does not claim a model generation"

# 2. The verifier.
(cd "$repo/provenance" && cargo build -q --locked --release) || die "verifier build failed"
vbin="${CARGO_TARGET_DIR:-$repo/provenance/target}/release/interplane-provenance"
policy_pin="$(sha256sum "$M5_POLICY" | cut -d' ' -f1)"
verify() { "$vbin" verify "$1" --judge-key "${2:-$M5_JUDGE_KEY}" --policy-sha256 "$policy_pin" 2>&1 | tail -1 || true; }
strong="effect=aien-ledger-slice/1:strong proposal=model_generation/2"
case "$(jq -r '.aien.native.claimed // "absent"' "$bundle/COMPANION.json")" in
  true) want="PASS complete $strong candidate=none";;
  *) want="PASS_LABELLED_INCOMPLETE missing=link:native $strong";;
esac
verdict="$(verify "$bundle")"
[ "$verdict" = "$want" ] || die "unexpected verdict: $verdict (wanted: $want)"
nokey="$("$vbin" verify "$bundle" 2>&1 | tail -1 || true)"
case "$nokey" in "FAIL evaluation_untrusted:"*) ;; *) die "without the judge key the bundle must be untrusted, got: $nokey";; esac
jq -e '.verdict == "PASS" and .test_exit_after == 0 and .test_exit_before != 0
  and .rollback.disk_sha256 == .target_blob_sha256_before' "$row/receipt.json" >/dev/null || die "row receipt does not show red-before, green-after and a restoring rollback"

# 3. Counterfeits of the model turn. Edits go to hard-linked copies (a changed file is written fresh).
neg="$OUT/negative"; mkdir -p "$neg"; : >"$neg/verdicts.txt"
fork() { rm -rf "$neg/$1"; cp -al "$bundle" "$neg/$1"; }
manifest() { local d="$1"; shift; jq "$@" "$d/COMPANION.json" >"$d/COMPANION.tmp" && mv "$d/COMPANION.tmp" "$d/COMPANION.json"; }
recpath() { jq -r ".records.$2.path" "$1/COMPANION.json"; }
restamp() { local d="$1" n="$2" p; p="$(recpath "$d" "$n")"
  manifest "$d" '.records[$n].sha256=$s | .records[$n].bytes=$b' --arg n "$n" \
    --arg s "$(sha256sum "$d/$p" | cut -d' ' -f1)" --argjson b "$(wc -c <"$d/$p")"; }
editrec() { local d="$1" n="$2" f="$3" p; shift 3; p="$(recpath "$d" "$n")"
  jq "$@" "$f" "$d/$p" >"$d/$p.new"; rm "$d/$p"; mv "$d/$p.new" "$d/$p"; restamp "$d" "$n"; }
expect() { # name bundle code
  local v; v="$(verify "$2")"
  case "$v" in "FAIL $3:"*) echo "negative $1: $v";; *) die "negative $1: wanted FAIL $3, got: $v";; esac
  echo "$1 $v" >>"$neg/verdicts.txt"; }
fork control; [ "$(verify "$neg/control")" = "$verdict" ] || die "negative control: an untouched copy no longer verifies"
echo "control $verdict" >>"$neg/verdicts.txt"
# Refused by the generation-record binding: the daemon record holds the real output digest.
fork resp;  editrec "$neg/resp" model_turn '.input = "<tool_call>\n{\"name\": \"write_file\", \"arguments\": {\"path\": \"README.md\", \"content\": \"# rsi-dashes\\n\"}}\n</tool_call>" | .response_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"'
expect response_text_replaced_refused_by_generation_record_binding "$neg/resp" binding_mismatch
fork wts;   editrec "$neg/wts" model_turn '.weights_sha256 = "9999999999999999999999999999999999999999999999999999999999999999"'
expect weights_digest_substituted "$neg/wts" binding_mismatch
fork bk;    editrec "$neg/bk" model_turn '.backend = "Backend: CUDA"'
expect backend_not_in_load_log "$neg/bk" binding_mismatch
fork reqid; editrec "$neg/reqid" model_turn '.request_sha256 = "9999999999999999999999999999999999999999999999999999999999999999"'
expect request_digest_substituted "$neg/reqid" binding_mismatch
fork gen;   editrec "$neg/gen" model_turn '.generation_record_id = 1'
expect generation_record_other "$neg/gen" binding_mismatch
fork nogen; manifest "$neg/nogen" 'del(.records.ledger_generation)'
expect generation_record_dropped "$neg/nogen" missing_record_entry
fork nomt;  manifest "$neg/nomt" 'del(.records.model_turn)'
v="$(verify "$neg/nomt")"; case "$v" in "FAIL "*) echo "negative model_turn_dropped: $v";; *) die "model_turn dropped must FAIL, got: $v";; esac
echo "model_turn_dropped $v" >>"$neg/verdicts.txt"
# The honest downgrade: the same bytes declared scripted can never be PASS complete.
fork scr; manifest "$neg/scr" '.effect.proposal_origin = "scripted_turn" | .completeness = {state: "incomplete", missing: ["link:model_turn"]}'
v="$(verify "$neg/scr")"; case "$v" in "PASS_LABELLED_INCOMPLETE missing=link:model_turn"*"proposal=scripted_turn"*) echo "negative scripted_downgrade: $v";; *) die "scripted downgrade must be labelled incomplete, got: $v";; esac
echo "scripted_downgrade $v" >>"$neg/verdicts.txt"
fork scrc; manifest "$neg/scrc" '.effect.proposal_origin = "scripted_turn"'
expect scripted_but_declared_complete "$neg/scrc" unlabelled_missing
# A model turn for other bytes: the generated README replaced on disk-bound records is already
# covered by the ledger checks; here the approved content differs from what the model wrote.
fork subj; editrec "$neg/subj" evaluation_receipt '.binding.subject_sha256 = "9999999999999999999999999999999999999999999999999999999999999999"'
expect edited_subject "$neg/subj" evaluation_signature_bad

# 4. Receipt.
finished="$(date -u +%FT%TZ)"
dirty=false; [ -z "$(git -C "$repo" status --porcelain -- adapters provenance 2>/dev/null)" ] || dirty=true
jq -n \
  --arg adapter_commit "$(git -C "$repo" rev-parse HEAD)" --argjson adapter_dirty "$dirty" \
  --arg bin_sha "$(sha256sum "$AIEN_BIN" | cut -d' ' -f1)" --arg verifier_sha "$(sha256sum "$vbin" | cut -d' ' -f1)" \
  --arg judge_sha "$(sha256sum "$SPARK_RSI_DIR/spark-rsi-judge" | cut -d' ' -f1)" \
  --arg judge_pub "$(cat "$M5_JUDGE_KEY")" --arg policy_pin "$policy_pin" \
  --arg sc_rev "${SOVEREIGN_CORE_REV:-UNVERIFIED}" --arg verdict "$verdict" --arg nokey "$nokey" \
  --argjson run "$(jq '{task_id, proposal_id, request_id, model_turn, source_commit, tree_commit_after, test_exit_before, test_exit_after, target_blob_sha256_before, target_blob_sha256_after, evaluation_receipt_sha256, evaluation_policy_sha256, rollback: {disk_sha256: .rollback.disk_sha256, test_exit_after_rollback: .rollback.test_exit_after_rollback}}' "$row/receipt.json")" \
  --argjson chain "$(jq -n \
      --slurpfile mt "$bundle/$(jq -r .records.model_turn.path "$bundle/COMPANION.json")" \
      --slurpfile ev "$bundle/$(jq -r .records.evaluation_receipt.path "$bundle/COMPANION.json")" \
      --slurpfile gr "$bundle/$(jq -r .records.ledger_grant.path "$bundle/COMPANION.json")" \
      --slurpfile ak "$bundle/$(jq -r .records.ledger_ack.path "$bundle/COMPANION.json")" \
      --slurpfile ig "$bundle/$(jq -r .records.ledger_intent.path "$bundle/COMPANION.json")" \
      --slurpfile gn "$bundle/$(jq -r .records.ledger_generation.path "$bundle/COMPANION.json")" \
      '{model_id: $mt[0].model_id, weights_sha256: $mt[0].weights_sha256, backend: $mt[0].backend,
        request_sha256: $mt[0].request_sha256, response_sha256: $mt[0].response_sha256,
        generation_record_id: $mt[0].generation_record_id,
        generation_output_text_sha256: ($gn[0].text | fromjson | .output_text_sha256),
        generation_model_sha256: ($gn[0].text | fromjson | .model_sha256),
        stream_started: $mt[0].stream_started, stream_finished: $mt[0].stream_finished,
        grant_id: $gr[0].id, grant_content_sha256: ($gr[0].text | fromjson | .content_sha256 // null),
        evaluation_subject_sha256: $ev[0].binding.subject_sha256,
        intent_id: $ig[0].id, ack_id: $ak[0].id,
        ack_disk_sha256: ($ak[0].text | fromjson | .disk_sha256 // null)}')" \
  --arg restart_a "$(jq -c '{records_after_restart, records_after_replays, daemon_replay_state: .daemon_replay.state, second_intent}' "$OUT/rows/row12/receipt.json")" \
  --arg restart_b "$(jq -c '{adapter_state, intent_id, effects_after_restart, second_intent, restart_reconcile}' "$OUT/rows/row13/receipt.json")" \
  --arg negatives "$(cat "$neg/verdicts.txt")" --arg started "$started" --arg finished "$finished" \
  '{kind: "interplane-aien-m5-model-turn-receipt", adapter_commit: $adapter_commit, adapter_dirty: $adapter_dirty,
    aien_cli_sha256: $bin_sha, verifier_sha256: $verifier_sha, spark_rsi_judge_sha256: $judge_sha,
    judge_public_key: $judge_pub, operator_policy_sha256: $policy_pin, sovereign_core_rev: $sc_rev,
    backend: "CPU reference (as printed in the daemon log)",
    task: $run, chain: $chain, verdict: $verdict, verdict_without_judge_key: $nokey,
    restart_rows: {daemon_restart_replay: ($restart_a | fromjson? // $restart_a), restart_between_intent_and_ack: ($restart_b | fromjson? // $restart_b)},
    negative_checks: ($negatives | split("\n") | map(select(length>0))),
    started: $started, finished: $finished,
    limits: ["one model turn is not a learning system: a 1B instruction model, greedy decoding, one prompt (a worked example on another file is part of the retained request), no feedback loop",
             "several prompts were tried before this one (the model copied the placeholder; the model kept the dashes): only the run that landed is retained",
             "the request bytes and the timestamps are the harness own; the daemon generation record carries only the token-id digest of the prompt, which the offline verifier cannot recompute without the tokenizer",
             "the generation record is unsigned and the weights digest is of the file at load time (hash-then-load gap), as in provenance/REAL-CHAIN.md",
             "the judge key is readable by the operator account (sudo): separation holds against the proposer, not the operator",
             "the holdout cases are in this public repository: integrity is pinned by digest, secrecy is not claimed",
             "the speed allowance of the pinned policy is 200 percent (see the committed scripted M5 evidence)",
             "CPU evidence only, not a frozen candidate (candidate_id null), not the minted flow"]}' >"$OUT/receipt.json"
echo "M5 MODEL-TURN GATE PASS: verdict: $verdict; receipt $OUT/receipt.json"
