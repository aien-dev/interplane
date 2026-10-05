#!/usr/bin/env bash
# Offline INTERPLANE checks, mirroring .github/workflows/ci.yml at the checked-out commit. No model, no GPU.
WT=$HOME/workspace/cand2-campaign/wt-ip-offline
OUT=$HOME/workspace/evidence-out/IP-OFFLINE-3fa8ee0
SC=$HOME/workspace/evidence-out/ip-scratch   # scratch: venvs, siblings, tmp (not published)
mkdir -p "$SC" "$OUT/logs"; cd "$WT" || exit 1
: > "$OUT/steps.tsv"
step() { # name, working-dir (relative to WT), command...
  local n=$1 d=$2; shift 2
  local s e rc; s=$(date -u +%FT%TZ)
  ( cd "$WT/$d" && bash -c "$*" ) > "$OUT/logs/$n.log" 2>&1; rc=$?
  e=$(date -u +%FT%TZ)
  printf '%s\t%s\t%s\t%s\t%s\n' "$n" "$rc" "$s" "$e" "$*" >> "$OUT/steps.tsv"
  echo "$n rc=$rc"
}
V=$SC/venv
python3 -m venv $V >/dev/null 2>&1
step pip-install 	. "$V/bin/pip install -q -c constraints/ci-python.txt pytest jsonschema pyflakes -e python"
PY="$V/bin/python"; export PATH="$V/bin:$PATH"
step schema-selfcheck . "$PY - <<'PYX'
import json,glob
from jsonschema import Draft202012Validator
for f in sorted(glob.glob('spec/schemas/*.json')):
    Draft202012Validator.check_schema(json.load(open(f))); print('ok', f)
PYX"
step fixture-validation . "$PY conformance/runners/validate_fixtures.py"
step bench-validate . "$PY bench/tools/validate.py --require-jsonschema"
step bench-validate-0.2x . "$PY bench/tools/validate.py --corpus 0.2x --require-jsonschema"
step live-injection-offline . "PYTHONPATH=python PYTHONDONTWRITEBYTECODE=1 $PY bench/tools/test_live_injection.py"
step stats-newcombe . "$PY bench/tools/stats.py"
step analyze-tests . "$PY bench/tools/test_analyze.py && rm -rf $SC/analyze-out && $PY bench/tools/analyze.py bench/tools/testdata/synthetic-run --out $SC/analyze-out && diff -r $SC/analyze-out bench/tools/testdata/synthetic-expected"
step rust-fmt rust "cargo fmt --all --check"
step rust-clippy rust "cargo clippy --all-targets --all-features -- -D warnings"
step rust-test rust "cargo test --all"
step rust-negctl-tests rust "cargo test -p interplane-conformance --features negative-controls"
step rust-conformance rust "cargo run -q -p interplane-conformance -- ../conformance/fixtures --out $OUT/rust-verdicts.json"
step py-pyflakes . "$PY -m pyflakes python/interplane python/tests"
step py-pytest . "$PY -m pytest python/tests -q"
step py-conformance . "$PY -m interplane.conformance conformance/fixtures --out $OUT/py-verdicts.json"
step cross-language . "$PY - <<'PYX'
import json,sys
r=json.load(open('$OUT/rust-verdicts.json')); p=json.load(open('$OUT/py-verdicts.json'))
bad=[c for c in sorted(set(r)|set(p)) if json.dumps(r.get(c),sort_keys=True)!=json.dumps(p.get(c),sort_keys=True)]
print('cases:',len(r),'mismatches:',bad); sys.exit(1 if bad or len(r)!=106 else 0)
PYX"
step trust-digest . "$PY conformance/runners/trust_digest.py --check"
step negctl-not-in-pipeline . "! grep -rIl -E 'negctl|negative-controls|NEGCTL' rust/crates/interplane-core rust/crates/interplane-crossveil rust/crates/interplane-crossaxis rust/crates/interplane-lenshift python/interplane/core.py python/interplane/crossveil.py python/interplane/crossaxis.py python/interplane/lenshift"
step default-build-refuses-variant rust "cargo build -q -p interplane-conformance && ! grep -q NEGCTL_BUILT_IN target/debug/interplane-conformance && { rc=0; target/debug/interplane-conformance ../conformance/fixtures --out /dev/null --variant V4 || rc=\$?; test \"\$rc\" = 2; }"
step negctl-matrix . "(cd rust && cargo run -q -p interplane-conformance --features negative-controls -- ../conformance/fixtures --matrix $OUT/rust-matrix.json) && $PY -m interplane.conformance conformance/fixtures --matrix $OUT/py-matrix.json && cmp $OUT/rust-matrix.json $OUT/py-matrix.json"
# package job
step pkg-wheel . "rm -rf $SC/dist && $V/bin/pip wheel -q --no-deps -w $SC/dist ./python"
step pkg-venv-examples . "rm -rf $SC/v && python3 -m venv $SC/v && $SC/v/bin/pip install -q $SC/dist/interplane-*.whl && cd $SC && INTERPLANE_REQUIRE_INSTALLED=1 $SC/v/bin/python $WT/examples/offline/run_offline.py && INTERPLANE_REQUIRE_INSTALLED=1 $SC/v/bin/python $WT/examples/adapter/todo_adapter.py"
step pkg-wheel-vs-checkout . "(cd $SC && $SC/v/bin/interplane-conformance $WT/conformance/fixtures --out $SC/wheel-verdicts.json) && PYTHONPATH=python $PY -m interplane.conformance $WT/conformance/fixtures --out $SC/checkout-verdicts.json && cmp $SC/wheel-verdicts.json $SC/checkout-verdicts.json"
step pkg-adapter-runner . "H=$WT/bench/runs/home-check-20261005T1320Z; rm -rf $SC/home && mkdir $SC/home && cp \$H/authority.py \$H/table-entry.json $SC/home/ && printf 'from authority import RUNTIME, POLICY, make_pipeline, HomeAuthority\n\n\ndef make_authority(**kw):\n    return HomeAuthority(**kw)\n' > $SC/home/shim.py && cd $SC && R=\"$SC/v/bin/python $WT/conformance/runners/adapter_runner.py --module home/shim.py --entry home/table-entry.json\" && \$R --out home/verdicts.json && cmp home/verdicts.json \$H/verdicts.json && { if \$R --no-exposure-check --out home/negctl.json; then echo negctl-passed; exit 1; fi; cmp home/negctl.json \$H/verdicts-negctl.json; }"
step pkg-cargo-package rust "cargo package --workspace --locked"
# adapter-aien
step aien-siblings . "adapters/aien/setup-siblings.sh $HOME/workspace/cand2-campaign"
step aien-fmt adapters/aien "cargo fmt --all --check"
step aien-clippy adapters/aien "cargo clippy --all-targets -- -D warnings"
step aien-test adapters/aien "cargo test"
step aien-t4 adapters/aien "T4_OUT=$OUT/t4-verdicts.json cargo test --test t4_corpus -- --nocapture"
