#!/usr/bin/env bash
# Re-verify the committed M5 model-turn bundle offline.
# Usage: reverify.sh <model_dir>   (a folder holding model.safetensors, tokenizer.json, config.json)
# The committed bundle omits those three files (2.4 GB). This copies them (real copies, not symlinks)
# into a scratch copy of the bundle, refuses if any digest differs from the manifest, then runs the
# offline verifier and prints its verdict. Nothing in the repo is changed.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
model_dir="${1:?usage: reverify.sh <model_dir>}"
command -v jq >/dev/null || { echo "reverify: jq is required" >&2; exit 2; }
scratch="$(mktemp -d)"; trap 'rm -rf "$scratch"' EXIT
cp -r "$here/bundle" "$scratch/bundle"
mkdir -p "$scratch/bundle/records/export"
for pair in export_weights:model.safetensors export_tokenizer:tokenizer.json export_config:config.json; do
  rec="${pair%%:*}"; file="${pair#*:}"
  want="$(jq -r ".records.$rec.sha256" "$scratch/bundle/COMPANION.json")"
  dst="$scratch/bundle/$(jq -r ".records.$rec.path" "$scratch/bundle/COMPANION.json")"
  [ -f "$model_dir/$file" ] || { echo "reverify: REFUSED, missing $model_dir/$file" >&2; exit 1; }
  cp -L "$model_dir/$file" "$dst"
  got="$(sha256sum "$dst" | cut -d' ' -f1)"
  [ "$got" = "$want" ] || { echo "reverify: REFUSED, $file digest $got != manifest $want" >&2; exit 1; }
  echo "reverify: $file matches manifest ($want)"
done
# The judge public key and the operator policy pin are read from the committed receipt. For a real
# audit pin them out of band instead (see README.md).
jq -r .judge_public_key "$here/receipt.json" >"$scratch/judge.pub"
policy_pin="$(jq -r .operator_policy_sha256 "$here/receipt.json")"
vbin="${VERIFIER:-}"
if [ -z "$vbin" ]; then
  (cd "$repo/provenance" && cargo build -q --locked --release)
  vbin="${CARGO_TARGET_DIR:-$repo/provenance/target}/release/interplane-provenance"
fi
"$vbin" verify "$scratch/bundle" --judge-key "$scratch/judge.pub" --policy-sha256 "$policy_pin" 2>&1 | tail -1 | sed 's/^/reverify verdict: /'
