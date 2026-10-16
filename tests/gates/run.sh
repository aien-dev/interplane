#!/usr/bin/env bash
# tests/gates/run.sh: the one entry point for the local verification gates of interplane.
# Every step runs the SAME commands as the hosted job it mirrors (.github/workflows/ci.yml, jobs
# no-python, schemas-and-fixtures, rust, package, negative-controls, provenance, adapter-aien).
# Manifests under tests/gates/ name this script plus one step word, so no manifest needs shell
# expansion (ADR 0028 P6). Observations print as "AIEN-OBS <name> <json>".
#
# Two deliberate differences from the hosted runner, neither changes what is checked:
# - Files the hosted job writes beside the checkout (rust-verdicts.json, rust-matrix.json,
#   t4-verdicts.json) go to a private temp dir here, so the checkout stays clean while the gates
#   of one head run in parallel in it; the cmp against conformance/expected/ is unchanged.
# - Paths into cargo's target dir honour CARGO_TARGET_DIR (the Spark worker sets it).
#
# Sibling checkouts (adapter-aien): the hosted job runs adapters/aien/setup-siblings.sh, which
# clones aegis-runtime and aien-protocols at the commits in adapters/aien/PINS into
# <parent>/interplane-audit/, where adapters/aien/Cargo.toml expects them
# (../../../interplane-audit/aegis-runtime). The Spark worker provides ../aegis-runtime and
# ../aien-protocols (main, read-only) and the run directory is not writable, so this step builds
# the hosted layout in a private temp dir: a copy of this commit at <tmp>/interplane and local
# clones of the two siblings (no network) at <tmp>/interplane-audit, then runs the real
# setup-siblings.sh there, which checks out the PINS commits and checks the Cargo.toml revs.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
step="${1:?usage: tests/gates/run.sh <step>}"
obs() { printf 'AIEN-OBS %s %s\n' "$1" "$2"; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CARGO_TERM_COLOR=never
TARGET="${CARGO_TARGET_DIR:-$ROOT/rust/target}"

need_sibling() {
  local name="$1"
  if [ ! -d "../$name" ]; then
    echo "run.sh: ../$name is missing; the hosted job clones aien-dev/$name at adapters/aien/PINS" >&2
    exit 3
  fi
  obs "sibling_${name//-/_}_commit" "\"$(git -C "../$name" rev-parse HEAD 2>/dev/null || echo unknown)\""
}

case "$step" in
  no-python)
    # Hosted ci.yml job no-python (project rule, Drake 2026-10-15: no Python ships here).
    found=$(git ls-files -- ':(glob)**/*.py' ':(glob)**/*.pyi' ':(glob)**/*.pyx' ':(glob)**/*.ipynb' \
      ':(glob)**/pyproject.toml' ':(glob)**/requirements*.txt' ':(glob)**/setup.py' ':(glob)**/setup.cfg' \
      ':(glob)**/Pipfile' ':(glob)**/Pipfile.lock')
    # Scripts without a .py ending: a first line that runs Python.
    shebang=$(git ls-files -z | xargs -0 -r awk 'FNR == 1 { if ($0 ~ /^#!.*python/) print FILENAME; nextfile }')
    found=$(printf '%s\n%s\n' "$found" "$shebang" | sed '/^$/d')
    if [ -n "$found" ]; then
      printf 'Python files are tracked; this repository ships no Python:\n%s\n' "$found"
      obs python_files_found true
      exit 1
    fi
    obs python_files_found false
    ;;
  schemas-and-fixtures)
    cd rust
    cargo run -q --locked -p interplane-conformance --bin interplane-corpus -- schemas
    cargo run -q --locked -p interplane-conformance --bin interplane-corpus -- validate
    ;;
  rust)
    cd rust
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all
    # negative controls (test-only feature) detect their defects
    cargo test -p interplane-conformance --features negative-controls
    # both examples run offline and check their own story (exit 1 on any difference)
    cargo run -q -p interplane-crossveil --example offline
    cargo run -q -p interplane-crossveil --example todo_adapter
    # conformance verdicts equal the recorded reference (106 cases)
    cargo run -q --locked -p interplane-conformance --bin interplane-conformance -- ../conformance/fixtures --out "$tmp/rust-verdicts.json"
    cmp "$tmp/rust-verdicts.json" ../conformance/expected/verdicts.json
    test "$(jq length "$tmp/rust-verdicts.json")" = 106
    obs conformance_cases 106
    ;;
  package)
    # Release check: every crate packages and builds from its .crate alone.
    cd rust
    cargo package --workspace --locked
    ;;
  negative-controls)
    # trust corpus and protocol digests match conformance/TRUST-DIGEST.txt
    (cd rust && cargo run -q --locked -p interplane-conformance --bin interplane-corpus -- trust-digest --check)
    # pipeline libraries never mention the negative controls ("if", because set -e ignores a "!" command)
    if grep -rIl -E 'negctl|negative-controls|NEGCTL' rust/crates/interplane-core rust/crates/interplane-crossveil rust/crates/interplane-crossaxis rust/crates/interplane-lenshift; then
      echo "run.sh: a pipeline library mentions the negative controls" >&2; exit 1
    fi
    # default build refuses a variant and carries no variant code
    (
      cd rust
      cargo build -q -p interplane-conformance
      if grep -q NEGCTL_BUILT_IN "$TARGET/debug/interplane-conformance"; then
        echo "run.sh: the default build carries variant code" >&2; exit 1
      fi
      rc=0; "$TARGET/debug/interplane-conformance" ../conformance/fixtures --out /dev/null --variant V4 || rc=$?
      test "$rc" = 2
    )
    # negative-control matrix, offline, equals the recorded reference (every variant must be detected)
    (cd rust && cargo run -q -p interplane-conformance --bin interplane-conformance --features negative-controls -- ../conformance/fixtures --matrix "$tmp/rust-matrix.json")
    cmp "$tmp/rust-matrix.json" conformance/expected/negative-control-matrix.json
    ;;
  provenance)
    cd provenance
    cargo fmt --all --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    # fixture verdicts from the CLI
    test "$(cargo run -q --locked -- verify fixtures/synthetic-full-chain)" = "PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
    test "$(cargo run -q --locked -- verify fixtures/synthetic-fix-the-test)" = "PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
    test "$(cargo run -q --locked -- verify fixtures/synthetic-weak-receipt)" = "PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=record_effect_receipt/1:weak"
    test "$(cargo run -q --locked -- verify fixtures/real-waldo-aien-chain)" = "PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2 candidate=none"
    test "$(cargo run -q --locked -- verify fixtures/real-waldo-tokenizer-json)" = "PASS_LABELLED_INCOMPLETE missing=link:effect,link:interplane"
    test "$(cargo run -q --locked -- verify fixtures/real-waldo-smoke)" = "PASS_LABELLED_INCOMPLETE missing=link:aien,link:effect,link:interplane"
    ;;
  adapter-aien)
    need_sibling aegis-runtime
    need_sibling aien-protocols
    # The hosted layout, built privately: <tmp>/interplane (this commit) beside <tmp>/interplane-audit.
    mkdir -p "$tmp/interplane" "$tmp/interplane-audit"
    git archive HEAD | tar -x -C "$tmp/interplane"
    for sib in aegis-runtime aien-protocols; do
      git clone -q --no-checkout "../$sib" "$tmp/interplane-audit/$sib"
    done
    # The real hosted setup step: checks out the PINS commits (already in the siblings' history,
    # no network) and checks that the Cargo.toml git revs equal PINS.
    "$tmp/interplane/adapters/aien/setup-siblings.sh" "$tmp"
    . "$tmp/interplane/adapters/aien/PINS"
    test "$(git -C "$tmp/interplane-audit/aegis-runtime" rev-parse HEAD)" = "$AEGIS_RUNTIME_REV"
    test "$(git -C "$tmp/interplane-audit/aien-protocols" rev-parse HEAD)" = "$AIEN_PROTOCOLS_REV"
    obs aegis_runtime_pin "\"$AEGIS_RUNTIME_REV\""
    obs aien_protocols_pin "\"$AIEN_PROTOCOLS_REV\""
    cd "$tmp/interplane/adapters/aien"
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    # T4 subset through the real Pipeline and AIEN authority (bench/PROTOCOL-0.3.md section 2; no skips)
    T4_OUT="$tmp/t4-verdicts.json" cargo test --test t4_corpus -- --nocapture
    ;;
  *)
    echo "run.sh: unknown step '$step'" >&2
    exit 2
    ;;
esac
