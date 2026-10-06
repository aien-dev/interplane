//! Synthetic full-chain fixture generator and the record sealer.
//!
//! SYNTHETIC: every byte written by `write_synthetic` is made up here. The WALDO records follow
//! the documented schema-1 shapes (docs/OPENWALDO-BOM.md, docs/MODEL-EXPORTS.md and
//! internal/model/records.go at openwaldo/waldo 0fd421a) but carry only the fields this verifier
//! reads plus a few for orientation. One deliberate deviation, labelled in the manifest: the
//! export carries a Hugging Face `tokenizer.json`, which a real WALDO Hugging Face export does not
//! (it ships `tokenizer_config.json` plus custom tokenizer code; see the real fixture).

use crate::gojson::{sha256_hex, waldo_sha256};
use crate::COMPANION_FILE;
use serde_json::{json, Value};
use std::path::Path;

pub const TRACE_ID: &str = "trace-prov-01";
pub const REQUEST_ID: &str = "call-prov-01";
pub const OTHER_REQUEST_ID: &str = "call-prov-02";
const RUN_ID: &str = "5e7a0c1d2b3f4a59";
const CAND: &str = "CAND-SYNTH-0";
const EXE: &str = "aien-cli-native-release";

/// Knobs for negative fixtures that must stay internally consistent (all digests recomputed).
#[derive(Default, Clone)]
pub struct Opts {
    /// Write this `vocab_size` into config.json instead of the tokenizer's real size.
    pub config_vocab: Option<u64>,
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(v).expect("json");
    b.push(b'\n');
    b
}

fn put(dir: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent"))?;
    std::fs::write(p, bytes)
}

fn safetensors(tensor: &str, data: &[u8]) -> Vec<u8> {
    let header =
        json!({ tensor: {"dtype": "BF16", "shape": [8, 4], "data_offsets": [0, data.len()]} });
    let mut h = serde_json::to_vec(&header).expect("json");
    while !h.len().is_multiple_of(8) {
        h.push(b' ');
    }
    let mut out = (h.len() as u64).to_le_bytes().to_vec();
    out.extend_from_slice(&h);
    out.extend_from_slice(data);
    out
}

fn artifact(role: &str, path: &str, b: &[u8]) -> Value {
    json!({"role": role, "path": path, "sha256": sha256_hex(b), "bytes": b.len()})
}

/// The aien-cli effect receipt shape (`record_effect_receipt`, version 1). Digests here use JCS;
/// aien-cli uses `serde_json::to_vec`, which is the same bytes for these string-only values.
pub fn receipt_bytes(tool: &str, args: &Value, data: &Value, success: bool) -> Vec<u8> {
    let d = |v: &Value| {
        format!(
            "sha256:{}",
            sha256_hex(interplane_core::canonicalize(v).as_bytes())
        )
    };
    serde_json::to_vec_pretty(&json!({
        "version": 1, "tool": tool, "success": success,
        "arguments_digest": d(args), "result_digest": d(data),
        "policy": "default_sovereign_engine", "timestamp": "2026-10-06T00:00:01+00:00"
    }))
    .expect("json")
}

pub fn request_args(request_id: &str) -> Value {
    if request_id == REQUEST_ID {
        json!({"path": "notes/today.md", "content": "hello world"})
    } else {
        json!({"path": "notes/other.md", "content": "note append"})
    }
}

pub fn result_data(request_id: &str) -> Value {
    let a = request_args(request_id);
    json!({"path": a["path"], "bytes_written": a["content"].as_str().unwrap_or("").len()})
}

fn envelope(
    n: u32,
    parent: Option<String>,
    src: (&str, &str),
    dst: (&str, &str),
    payload: Value,
) -> Value {
    json!({
        "interplane_version": "0.1", "message_id": format!("m-{n}"), "trace_id": TRACE_ID,
        "parent_id": parent, "timestamp": format!("2026-10-06T00:00:0{n}Z"),
        "source": {"kind": src.0, "id": src.1}, "destination": {"kind": dst.0, "id": dst.1},
        "payload": payload
    })
}

fn trace() -> Value {
    let mut envs = Vec::new();
    let mut n = 0;
    for rid in [REQUEST_ID, OTHER_REQUEST_ID] {
        n += 1;
        let req = n;
        envs.push(envelope(
            n,
            None,
            ("model", "synthetic-notes"),
            ("runtime", "aien"),
            json!({
                "kind": "tool_request", "request_id": rid,
                "tool": {"namespace": null, "name": "write_file"},
                "arguments": request_args(rid),
                "provenance": {"dialect": "openai", "parser_version": "1"}
            }),
        ));
        n += 1;
        envs.push(envelope(n, Some(format!("m-{req}")), ("runtime", "aien"), ("model", "synthetic-notes"), json!({
            "kind": "result", "request_id": rid, "status": "ok", "data": result_data(rid),
            "error": null, "decision": null,
            "provenance": {"capability": "write_file", "content_kind": "tool_result", "duration_ms": null,
                           "runtime": "aien", "trust": "trusted_runtime", "trusted": true}
        })));
    }
    Value::Array(envs)
}

/// Write the synthetic full-chain archive (records + sealed COMPANION.json) into `dir`.
pub fn write_synthetic(dir: &Path, o: &Opts) -> std::io::Result<()> {
    // Export-side files.
    let names = [
        "<unk>", "<s>", "</s>", "hello", "world", "note", "append", "ok",
    ];
    let vocab: serde_json::Map<String, Value> = names
        .iter()
        .enumerate()
        .map(|(i, t)| (t.to_string(), json!(i)))
        .collect();
    let added: Vec<Value> = (0..3)
        .map(|i| json!({"id": i, "content": names[i], "special": true}))
        .collect();
    let tokenizer = pretty(&json!({
        "version": "1.0",
        "added_tokens": added,
        "pre_tokenizer": {"type": "Whitespace"},
        "model": {"type": "WordLevel", "vocab": vocab, "unk_token": "<unk>"}
    }));
    let config = pretty(&json!({
        "architectures": ["LlamaForCausalLM"], "model_type": "llama",
        "vocab_size": o.config_vocab.unwrap_or(names.len() as u64),
        "hidden_size": 4, "intermediate_size": 8, "num_hidden_layers": 1,
        "num_attention_heads": 1, "num_key_value_heads": 1, "max_position_embeddings": 16,
        "tie_word_embeddings": true, "torch_dtype": "bfloat16", "bos_token_id": 1, "eos_token_id": 2
    }));
    let chat = b"{% for m in messages %}<|{{ m.role }}|>\n{{ m.content }}</s>\n{% endfor %}<|assistant|>\n".to_vec();
    let data: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(37)).collect();
    let run_w = safetensors("tok_embeddings.weight", &data);
    let exp_w = safetensors("model.embed_tokens.weight", &data);

    // WALDO training side.
    let corpus = json!({
        "kind": "openwaldo-bom", "schema": 1, "subject": "corpus",
        "index": {"remote": "synthetic", "commit": "0".repeat(40), "dirty": false},
        "paths": ["synthetic/notes"], "license_policy": {}, "manifests": [], "shards": [],
        "totals": {"files": 1, "docs": 1, "tokens": 20, "bytes": 64},
        "licenses": {"CC0-1.0": {"shards": 1, "docs": 1, "tokens": 20, "bytes": 64}}
    });
    let plan = pretty(
        &json!({"kind": "waldo-model-plan", "schema": 1, "name": "synthetic-notes",
        "architecture_sha256": sha256_hex(b"synthetic-architecture")}),
    );
    let model_id = waldo_sha256(&plan);
    let preflight = pretty(&json!({"kind": "waldo-preflight", "schema": 1, "held_out_rows": []}));
    let backend = json!({"name": "pytorch", "revision": "synthetic-worker-r0"});
    let run_bom = pretty(&json!({
        "kind": "openwaldo-bom", "schema": 1, "subject": "training-run", "id": RUN_ID,
        "model_id": model_id, "stage": "pretrain", "objective": "causal-lm",
        "execution": {"backend": backend, "framework": "pytorch"},
        "corpus_bom_sha256": sha256_hex(serde_json::to_string(&corpus).expect("json").as_bytes()),
        "corpus_bom": corpus,
        "preflight": {"path": "PREFLIGHT.json", "sha256": sha256_hex(&preflight), "bytes": preflight.len()}
    }));
    let run_dir = format!("runs/0001-pretrain-{RUN_ID}/artifacts");
    let run = pretty(
        &json!({"kind": "waldo-training-run", "schema": 1, "id": RUN_ID,
        "state": "complete", "bom_sha256": waldo_sha256(&run_bom)}),
    );
    let model_bom = pretty(&json!({
        "kind": "openwaldo-bom", "schema": 1, "subject": "model", "model_id": model_id,
        "name": "synthetic-notes", "plan_sha256": model_id, "path_base": "model-root",
        "current_run_id": RUN_ID,
        "runs": [{"id": RUN_ID, "stage": "pretrain", "ordinal": 1, "bom_sha256": waldo_sha256(&run_bom),
                  "state": "complete", "backend": backend, "simulated": false,
                  "artifacts": [artifact("weights", &format!("{run_dir}/model.safetensors"), &run_w)]}]
    }));
    let release = pretty(&json!({
        "kind": "openwaldo-bom", "schema": 1, "subject": "model-release", "format": "huggingface",
        "model_id": model_id, "name": "synthetic-notes", "source_type": "run", "source_id": RUN_ID,
        "run_id": RUN_ID, "source_bom_sha256": waldo_sha256(&model_bom),
        "artifacts": [artifact("interaction-template", "chat_template.jinja", &chat),
                      artifact("configuration", "config.json", &config),
                      artifact("weights", "model.safetensors", &exp_w),
                      artifact("tokenizer", "tokenizer.json", &tokenizer)],
        "generated": "2026-10-06T00:00:00Z"
    }));

    // AIEN side: candidate manifest (CandidateManifestV1 subset) and the daemon load line.
    let exe_sha = sha256_hex(b"synthetic aien-cli executable");
    let cand = format!(
        "# SYNTHETIC candidate manifest, CandidateManifestV1 subset (aien-architecture qualification/candidates).\n\
         schema = \"CandidateManifestV1\"\nid = \"{CAND}\"\nstatus = \"synthetic\"\n\n[executables]\n{EXE} = \"{exe_sha}\"\n\n\
         [model]\nmodel-id = \"synthetic-notes\"\nmodel-safetensors-sha256 = \"{}\"\ntokenizer-json-sha256 = \"{}\"\nconfig-json-sha256 = \"{}\"\n",
        sha256_hex(&exp_w), sha256_hex(&tokenizer), sha256_hex(&config)
    );
    let load = format!(
        "checkpoint loaded from /models/synthetic-notes/model.safetensors (tokenizer loaded, model_id=synthetic-notes, config=synthetic-notes (config.json), model_sha256={}, tokenizer_sha256={})\n",
        sha256_hex(&exp_w), sha256_hex(&tokenizer)
    );
    let trace = pretty(&trace());
    let receipt = receipt_bytes(
        "write_file",
        &request_args(REQUEST_ID),
        &result_data(REQUEST_ID),
        true,
    );

    let files: Vec<(&str, &str, Vec<u8>)> = vec![
        ("waldo_plan", "records/waldo/PLAN.json", plan),
        ("waldo_model_bom", "records/waldo/MODEL-BOM.json", model_bom),
        ("waldo_run_bom", "records/waldo/RUN-BOM.json", run_bom),
        ("waldo_run", "records/waldo/RUN.json", run),
        ("waldo_preflight", "records/waldo/PREFLIGHT.json", preflight),
        ("run_weights", "records/waldo/model.safetensors", run_w),
        ("release_bom", "records/export/BOM.json", release),
        ("export_weights", "records/export/model.safetensors", exp_w),
        ("export_config", "records/export/config.json", config),
        (
            "export_tokenizer",
            "records/export/tokenizer.json",
            tokenizer,
        ),
        (
            "export_chat_template",
            "records/export/chat_template.jinja",
            chat,
        ),
        (
            "candidate_manifest",
            "records/aien/CAND-SYNTH-0.toml",
            cand.into_bytes(),
        ),
        (
            "aien_load_log",
            "records/aien/daemon-load.log",
            load.into_bytes(),
        ),
        ("interplane_trace", "records/interplane/trace.json", trace),
        (
            "effect_receipt",
            "records/aien/effect-receipt.json",
            receipt,
        ),
    ];
    let mut records = serde_json::Map::new();
    for (name, rel, bytes) in &files {
        put(dir, rel, bytes)?;
        records.insert(name.to_string(), json!({"path": rel}));
    }
    let skeleton = json!({
        "kind": crate::KIND, "schema": crate::SCHEMA,
        "fixture": {"class": "synthetic", "generator": "provenance/src/synth.rs",
            "note": "Every byte is synthetic. The export carries a Hugging Face tokenizer.json, which a real WALDO Hugging Face export does not; no AIEN process loaded this model and no runtime wrote this receipt."},
        "completeness": {"state": "complete", "missing": []},
        "records": records,
        "lineage": {"kind": "waldo_run", "run_id": RUN_ID, "model_id": model_id,
            "corpus_license_assertions": ["CC0-1.0"], "training_backend": backend,
            "training_code": {"repository": "github.com/openwaldo/waldo", "commit": "synthetic", "checked": false}},
        "export": {"format": "huggingface", "model_revision": format!("{model_id}/{RUN_ID}"),
            "chat_template": "export_chat_template",
            "conversion_tool": {"name": "waldo model export --format huggingface", "commit": "synthetic", "checked": false}},
        "load_support": {"declared": crate::SUPPORTED},
        "aien": {"candidate_id": CAND, "executable": EXE, "executable_sha256": exe_sha},
        "interplane": {"trace_id": TRACE_ID, "request_id": REQUEST_ID},
        "effect": {"receipt": "effect_receipt"}
    });
    seal(dir, skeleton)
}

/// Fill `sha256`, `bytes` (and `waldo_sha256` for WALDO JSON records) of every retained record
/// from the files under `dir`, then write COMPANION.json. Refuses to overwrite an existing
/// manifest: a sealed record is never rewritten to match a new result.
pub fn seal(dir: &Path, mut skeleton: Value) -> std::io::Result<()> {
    let out = dir.join(COMPANION_FILE);
    if out.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "COMPANION.json exists; never rewritten",
        ));
    }
    if let Some(recs) = skeleton.get_mut("records").and_then(Value::as_object_mut) {
        for (name, r) in recs.iter_mut() {
            if r.get("retained") == Some(&Value::Bool(false)) {
                continue;
            }
            let rel = r["path"].as_str().unwrap_or("").to_string();
            let b = std::fs::read(dir.join(&rel))?;
            r["sha256"] = json!(sha256_hex(&b));
            r["bytes"] = json!(b.len());
            let waldo_json = matches!(
                name.as_str(),
                "waldo_plan" | "waldo_model_bom" | "waldo_run_bom" | "waldo_run" | "release_bom"
            );
            if waldo_json {
                r["waldo_sha256"] = json!(waldo_sha256(&b));
            }
        }
    }
    std::fs::write(out, pretty(&skeleton))
}
