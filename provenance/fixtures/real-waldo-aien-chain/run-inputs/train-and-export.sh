#!/bin/sh
# Train the tool-call model: structured conversations -> SFT with chatml-v1 -> Hugging Face export.
set -eu
T=$HOME/.claude/jobs/6c731f18/tmp/ipx76; S=$HOME/.claude/jobs/bb9e2577/tmp/waldo29
export PATH=$S/venvB/bin:$S/tools/go/bin:$PATH GOPATH=$S/gopath GOCACHE=$S/gocache GOFLAGS=-buildvcs=false
W=$T/train2/work; rm -rf $W; mkdir -p $W/source/raw
binary=$W/waldo; (cd $T/waldo && go build -o $binary ./cmd/waldo)
export WALDO_CONFIG=$W/config.json
cp $T/train2/conversations.jsonl $W/source/raw/conversations.jsonl
fb=$(wc -c < $W/source/raw/conversations.jsonl | tr -d ' ')
fs=$(sha256sum $W/source/raw/conversations.jsonl | awk '{print $1}')
tree=$(printf '%s\t%s\t%s\n' "$fs" "$fb" conversations.jsonl | sha256sum | awk '{print $1}')
cat > $W/source/manifest.json <<EOM
{"kind":"waldo-source-directory","schema":1,"retrieved_at":"2026-10-07T00:00:00Z",
 "corpus":{"id":"aien-toolcall-own","title":"AIEN tool-call conversations","description":"Own tiny corpus: write_file tool calls, written for the provenance fixture"},
 "sources":[{"id":"aien-toolcall-own","path":"","license":"CC0-1.0",
  "source":{"name":"aien-toolcall-own","version":"1","url":"https://example.invalid/aien-toolcall-own","category":"public-dataset","license_evidence":{"declaration":"CC0-1.0"}},
  "input":{"format":"jsonl","type":"chat-messages","fields":{"id":"id"},"messages":{"role":"conversations[].from","content":"conversations[].value","role_aliases":{"human":"user","gpt":"assistant"}}},
  "artifacts":[]}],
 "fetcher":{"name":"aien-toolcall-own"},
 "raw":{"path":"raw","file_count":1,"byte_count":$fb,"tree_sha256":"$tree"}}
EOM
$binary index init $W/idx >/dev/null
$binary config set index $W/idx >/dev/null
$binary config set lookaside file://$W/look >/dev/null
$binary config set lookaside.cache $W/cache >/dev/null
$binary config set lookaside.cache.max-size 64MiB >/dev/null
$binary config set lookaside.scratch $W/scratch >/dev/null
$binary config set ingest.staging $W/staging >/dev/null
$binary config set model.root $W/models >/dev/null
$binary config set model.backend pytorch >/dev/null
cat > $W/provider.json <<EOP
{"kind":"waldo-disclosure-provider","schema":1,"provider":{"name":"AIEN provenance fixture","address":"Local","contact":"test@example.invalid"},"code_of_practice_status":"not-assessed","copyright_policy_url":"https://example.invalid/copyright"}
EOP
$binary config set disclosure.provider $W/provider.json >/dev/null
dest=$W/idx/post-train/sft/aien-toolcall-own
$binary index ingest $W/source $dest >/dev/null
c=; for d in $W/staging/*/contribution; do [ -d $d ] && c=$d; done
cp -R $c/. $W/idx/
cat > $W/model.yaml <<EOY
kind: waldo-model-compose
schema: 1
interaction:
  template: chatml-v1
architecture:
  family: decoder-transformer
  context_tokens: 2048
  vocabulary_size: 259
  hidden_size: 128
  intermediate_size: 256
  layers: 2
  attention_heads: 4
  key_value_heads: 2
  tie_embeddings: true
  parameter_dtype: bfloat16
  tokenizer:
    name: byte
    revision: builtin-byte-schema-1
stages:
  - name: toolcall-sft
    type: fine-tuning
    objective: assistant-response-modeling
    conversation:
      template: chatml-v1
      supervised_roles: [assistant]
    corpora:
      - post-train/sft/aien-toolcall-own
    parameters:
      epochs: ${EPOCHS:-30}
      batch_size: 4
      sequence_length: 256
      learning_rate: ${LR:-0.003}
      seed: 7
      evaluation_max_records: 1
      evaluation_max_bytes: 1048576
EOY
$binary model train toolcall-tiny $W/model.yaml --audit
$binary model export toolcall-tiny $W/hf --format huggingface --allow-incomplete
ls $W/hf
