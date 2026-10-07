#!/bin/sh
set -eu
T=$HOME/.claude/jobs/6c731f18/tmp/ipx76; S=$HOME/.claude/jobs/bb9e2577/tmp/waldo29
export PATH=$S/venvB/bin:$S/tools/go/bin:$PATH GOPATH=$S/gopath GOCACHE=$S/gocache GOFLAGS=-buildvcs=false
W=$T/train/work; rm -rf $W; mkdir -p $W
binary=$W/waldo; (cd $T/waldo && go build -o $binary ./cmd/waldo)
export WALDO_CONFIG=$W/config.json
$binary index init $W/idx >/dev/null
$binary config set lookaside file://$W/look >/dev/null
$binary config set lookaside.cache $W/cache >/dev/null
$binary config set lookaside.scratch $W/scratch >/dev/null
$binary config set ingest.staging $W/staging >/dev/null
$binary config set model.root $W/models >/dev/null
$binary config set model.backend pytorch >/dev/null
$binary config set index $W/idx >/dev/null
cat > $W/provider.json <<EOP
{"kind":"waldo-disclosure-provider","schema":1,"provider":{"name":"AIEN provenance fixture","address":"Local","contact":"test@example.invalid"},"code_of_practice_status":"not-assessed","copyright_policy_url":"https://example.invalid/copyright"}
EOP
$binary config set disclosure.provider $W/provider.json >/dev/null
$binary index ingest $T/train/corpus.txt $W/idx/core/e2e/prov --title Provenance-fixture-corpus --description Own-tiny-corpus --license CC0-1.0 --source https://example.invalid/own --language en --source-category public-dataset >/dev/null
c=; for d in $W/staging/*/contribution; do [ -d $d ] && c=$d; done
cp -R $c/. $W/idx/
cat > $W/model.yaml <<EOY
kind: waldo-model-compose
schema: 1
interaction:
  template: chatml-v1
architecture:
  family: decoder-transformer
  context_tokens: 1024
  vocabulary_size: 259
  hidden_size: 32
  intermediate_size: 64
  layers: 1
  attention_heads: 4
  key_value_heads: 2
  tie_embeddings: true
  parameter_dtype: bfloat16
  tokenizer:
    name: byte
    revision: builtin-byte-schema-1
stages:
  - name: pretrain
    type: pre-training
    objective: causal-language-modeling
    corpora:
      - core/e2e/prov
    parameters:
      steps: 10
      batch_size: 2
      sequence_length: 64
      learning_rate: 0.003
      seed: 7
      checkpoint_every: 5
      evaluate_every: 5
EOY
$binary model train prov-tiny $W/model.yaml
$binary model export prov-tiny $W/hf --format huggingface --allow-incomplete
ls -la $W/hf
