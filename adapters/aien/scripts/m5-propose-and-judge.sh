#!/usr/bin/env bash
# VAC M5 hook: `m5-propose-and-judge.sh <task-workspace> <out-dir>`, run by the live row
# `m5_rsi_judged_change_lands_and_rolls_back`. Writes <out>/proposal.json, receipt.json and
# policy.json. It decides nothing: the harness checks the receipt before anything reaches the
# daemon, and the daemon's normal approval applies after that.
#
# 1. The RSI engine runs as RSI_USER on a read-only copy of the workspace and prints one proposal.
# 2. A proposal for any file other than README.md is refused here.
# 3. The operator builds the candidate tree: the parent copy with the proposed README.
# 4. The judge runs as JUDGE_USER on its own copies (parent, candidate, holdouts, policy, its own
#    binary) inside its mode-700 home, builds both trees itself into a fresh target directory,
#    runs the holdouts in the jail and signs a version 2 receipt with its own key.
#
# Environment: SPARK_RSI_DIR (holds release `spark-rsi` and `spark-rsi-judge`), M5_POLICY (the
# pinned policy file), M5_HOLDOUTS (holdout suite directory); JUDGE_USER, RSI_USER, JUDGE_HOME.
# Negative tests only (the gate script uses these to make counterfeit evidence, never the row):
# M5_SUBJECT_OVERRIDE=<file> judges that file as the README instead of the proposal;
# M5_JUDGE_KEY_OVERRIDE=<file> signs with that key instead of the judge's own;
# M5_RECEIPT_V1=1 asks for a version 1 receipt (no binding). All three are ignored unless
# M5_GATE_NEGATIVE=1 (the gate sets it for its negatives; the live row strips all four).
# Exit 0 when a receipt was written (admitted or not: the harness reads it), non-zero otherwise.
set -euo pipefail
die() { echo "M5 HOOK FAIL: $*" >&2; exit 1; }
ws="${1:?workspace}"; out="${2:?out dir}"
: "${SPARK_RSI_DIR:?}" "${M5_POLICY:?}" "${M5_HOLDOUTS:?}"
judge_user="${JUDGE_USER:-aien-judge}"
rsi_user="${RSI_USER:-aien-rsi}"
judge_home="${JUDGE_HOME:-/var/lib/aien-judge}"
target="README.md"
as_judge() { sudo -n -u "$judge_user" "$@"; }
command -v jq >/dev/null || die "jq is required"
if [ "${M5_GATE_NEGATIVE:-0}" != 1 ]; then
  unset M5_SUBJECT_OVERRIDE M5_JUDGE_KEY_OVERRIDE M5_RECEIPT_V1
fi
# The judged parent is the workspace's commit, exported by git itself (so ignored, untracked or
# flagged files in the folder never reach the judge). Git runs with no hook or fsmonitor program
# from the workspace's own config.
g() { git -c core.fsmonitor= -c core.hooksPath=/dev/null -c core.attributesFile=/dev/null -C "$ws" "$@"; }
[ "$(g rev-parse --show-toplevel 2>/dev/null)" = "$(cd "$ws" && pwd -P)" ] \
  || die "workspace is not the top of its own git repository: refused"
ws_status="$(g status --porcelain --untracked-files=all)" || die "cannot read the workspace status"
[ -z "$ws_status" ] || die "workspace has uncommitted changes: the judged parent would not be its commit"
parent_commit="$(g rev-parse --verify 'HEAD^{commit}')" || die "workspace has no commit"
# Attributes could make the export differ from the commit (export-ignore, export-subst).
tree_files="$(g ls-tree -r --name-only "$parent_commit")" || die "cannot list the task commit"
! grep -Eq '(^|/)\.gitattributes$' <<<"$tree_files" || die "the task commit carries .gitattributes: refused"
[ ! -s "$(g rev-parse --path-format=absolute --git-path info/attributes)" ] || die "the workspace has info/attributes: refused"

export_commit() { # dst: the parent commit's tree, exactly
  mkdir -p "$1"; g archive --format=tar "$parent_commit" | tar -C "$1" -xf -
}
copy_tree() { # src dst: a staged tree
  mkdir -p "$2"; tar -C "$1" -cf - . | tar -C "$2" -xf -
}

stage="$(mktemp -d /tmp/aien-m5.XXXXXX)"
trap 'chmod -R u+w "$stage" 2>/dev/null; rm -rf "$stage"' EXIT
chmod 755 "$stage"
mkdir "$stage/bin"
cp "$SPARK_RSI_DIR/spark-rsi" "$stage/bin/spark-rsi"
export_commit "$stage/view"
[ -z "$(find "$stage/view" -type l -print -quit)" ] || die "the task commit contains a symlink: refused"
chmod -R a+rX,a-w "$stage/view" "$stage/bin"

# 1. Propose, as the RSI account, from a read-only view.
sudo -n -u "$rsi_user" env -i HOME=/nonexistent PATH=/usr/bin:/bin \
  "$stage/bin/spark-rsi" propose "$stage/view" >"$out/proposal.json" \
  || die "spark-rsi propose failed"
# 2. Only the task's file.
got="$(jq -r '.target_file // "none"' "$out/proposal.json")"
[ "$got" = "$target" ] || die "RSI proposed a change to $got; only $target is allowed: refused"

# 3. Candidate = parent + the proposed file, byte for byte.
copy_tree "$stage/view" "$stage/candidate"
chmod -R u+w "$stage/candidate"
jq -e '.proposed_patch | type == "string"' "$out/proposal.json" >/dev/null \
  || die "the proposal carries no whole-file content"
jq -j '.proposed_patch' "$out/proposal.json" >"$stage/candidate/$target"
if [ -n "${M5_SUBJECT_OVERRIDE:-}" ]; then cp "$M5_SUBJECT_OVERRIDE" "$stage/candidate/$target"; fi
chmod -R a+rX "$stage/candidate"

# 4. The judge, on its own copies.
run="m5-$(date -u +%Y%m%dT%H%M%SZ)-$$"
jdir="$judge_home/m5/$run"
as_judge mkdir -p "$jdir/parent" "$jdir/candidate" "$jdir/holdouts" "$jdir/out"
send() { tar -C "$1" -cf - . | as_judge tar -C "$2" -xf -; }
send "$stage/view" "$jdir/parent"
send "$stage/candidate" "$jdir/candidate"
send "$M5_HOLDOUTS" "$jdir/holdouts"
as_judge sh -c 'cat > "$1"' _ "$jdir/policy.json" <"$M5_POLICY"
as_judge sh -c 'cat > "$1" && chmod 700 "$1"' _ "$jdir/spark-rsi-judge" <"$SPARK_RSI_DIR/spark-rsi-judge"
candidate_id="$(jq -r .id "$out/proposal.json")"
keyfile="$judge_home/judge.key"
if [ -n "${M5_JUDGE_KEY_OVERRIDE:-}" ]; then
  keyfile="$jdir/override.key"
  as_judge sh -c 'umask 077; cat > "$1"' _ "$keyfile" <"$M5_JUDGE_KEY_OVERRIDE"
fi
bind=(--subject-path "$target" --policy-file "$jdir/policy.json")
want='.format_version == 2 and (.signature | type == "string")'
if [ "${M5_RECEIPT_V1:-0}" = 1 ]; then
  bind=(); want='(.format_version // 1) == 1 and (.signature | type == "string")'
fi
set +e
as_judge env -i HOME="$judge_home" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
  CARGO_HOME="$judge_home/.cargo" PATH="${CARGO_BIN_DIR:-$HOME/.cargo/bin}:/usr/bin:/bin" \
  "$jdir/spark-rsi-judge" --cycle-id "$run" --candidate-id "$candidate_id" \
  --parent-id "$parent_commit" --candidate-path "$jdir/candidate" --parent-path "$jdir/parent" \
  --holdouts-dir "$jdir/holdouts" --output-dir "$jdir/out" \
  --signing-key-file "$keyfile" "${bind[@]}" --executable rsi-dashes --build-release \
  >"$out/receipt.json" 2>"$out/judge.stderr"
rc=$?
set -e
jq -e "$want" "$out/receipt.json" >/dev/null 2>&1 \
  || die "the judge wrote no receipt of the expected version (exit $rc): $(tail -3 "$out/judge.stderr")"
cp "$M5_POLICY" "$out/policy.json"
echo "M5 HOOK: judge exit $rc, receipt $out/receipt.json (judge dir $jdir)"
