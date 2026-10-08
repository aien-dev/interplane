//! Synthetic full-chain fixture generator and the record sealer.
//!
//! SYNTHETIC: every byte written by `write_synthetic` is made up here. The WALDO records follow
//! the documented schema-1 shapes (docs/OPENWALDO-BOM.md, docs/MODEL-EXPORTS.md and
//! internal/model/records.go at openwaldo/waldo 0fd421a) but carry only the fields this verifier
//! reads plus a few for orientation. One deliberate deviation, labelled in the manifest: the
//! export carries a Hugging Face `tokenizer.json`, which a real WALDO Hugging Face export does not
//! (it ships `tokenizer_config.json` plus custom tokenizer code; see the real fixture).

use crate::binding::{
    approval_binding_bytes, approved_proposal_bytes, compact_sorted, compose_proposal_bytes,
    ApprovalFields, APPROVAL_BINDING, LEDGER_BINDING, RECEIPT_BINDING, SERDE_DIGEST_FORM,
};
use crate::gojson::{sha256_hex, waldo_sha256};
use crate::COMPANION_FILE;
use serde_json::{json, Value};
use std::path::Path;

pub const TRACE_ID: &str = "trace-prov-01";
pub const REQUEST_ID: &str = "call-prov-01";
pub const OTHER_REQUEST_ID: &str = "call-prov-02";
/// Same tool, arguments and result data as `REQUEST_ID`, in the same trace.
pub const IDENTICAL_REQUEST_ID: &str = "call-prov-03";
pub const WORKSPACE: &str = "/synthetic/workspace";
pub const DAEMON_PID: u64 = 4242;
pub const DAEMON_START_TICKS: u64 = 777_000;
const RUN_ID: &str = "5e7a0c1d2b3f4a59";
const CAND: &str = "CAND-SYNTH-0";
const EXE: &str = "aien-cli-native-release";

/// Knobs for negative fixtures that must stay internally consistent (all digests recomputed).
#[derive(Default, Clone)]
pub struct Opts {
    /// Write this `vocab_size` into config.json instead of the tokenizer's real size.
    pub config_vocab: Option<u64>,
    /// Trace id of the INTERPLANE trace and the ledger (default [`TRACE_ID`]).
    pub trace_id: Option<String>,
    /// The request the effect link names (default [`REQUEST_ID`]).
    pub request_id: Option<String>,
    /// Write the weaker `record_effect_receipt/1` binding instead of the ledger slice.
    pub weak: bool,
    /// Write the fix-the-test slice: a write that replaces `src/clamp.c`, the `vac-test-run/1`
    /// records and a `compose_recall` report (see [`fix_the_test`]).
    pub fix_the_test: bool,
    /// With `fix_the_test`: the native-claim variant of the recall report (default: no `aien.native`).
    pub native: Option<bool>,
}

/// Arguments and prior digest of a write that replaces an existing file.
#[derive(Clone)]
pub struct Over {
    pub args: Value,
    pub prior_sha256: String,
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

/// The aien-cli effect receipt shape (`record_effect_receipt`, version 1). Digests use the
/// producer's form (`serde_json.to_vec.sorted-keys/1`, see binding.rs).
pub fn receipt_bytes(tool: &str, args: &Value, data: &Value, success: bool) -> Vec<u8> {
    let d = |v: &Value| {
        format!(
            "sha256:{}",
            sha256_hex(&compact_sorted(v).expect("integers only"))
        )
    };
    serde_json::to_vec_pretty(&json!({
        "version": 1, "tool": tool, "success": success,
        "arguments_digest": d(args), "result_digest": d(data),
        "policy": "default_sovereign_engine", "timestamp": "2026-10-06T00:00:01+00:00"
    }))
    .expect("json")
}

/// Requests 01 and 03 are identical calls; 02 differs.
pub fn request_args(request_id: &str) -> Value {
    if request_id == OTHER_REQUEST_ID {
        json!({"path": "notes/other.md", "content": "note append"})
    } else {
        json!({"path": "notes/today.md", "content": "hello world"})
    }
}

pub fn result_data(request_id: &str) -> Value {
    let a = request_args(request_id);
    json!({"path": a["path"], "bytes_written": a["content"].as_str().unwrap_or("").len()})
}

/// One Cortex record as `ComposeRecall` exports it (`ComposeRecordView`). SYNTHETIC digest.
fn record_view(id: u64, links: [u64; 4], note: &str, text: &Value) -> Value {
    let text = text.to_string();
    json!({"id": id, "cls": 0, "kind": 0, "subject": 0, "tag": 0, "links": links,
        "digest": sha256_hex(format!("synthetic-record:{id}:{text}").as_bytes()),
        "verified": true, "note": note, "text": text})
}

/// The five daemon-written ledger records of one approved write, SYNTHETIC but built through the
/// same binding functions the verifier uses. `index` spaces the record ids apart.
pub fn ledger_slice(trace_id: &str, request_id: &str, index: u64) -> Vec<(&'static str, Value)> {
    ledger_slice_over(trace_id, request_id, index, None)
}

/// [`ledger_slice`] for a write that replaces an existing file: `over` names the arguments and the
/// digest of the bytes the file held before.
pub fn ledger_slice_over(
    trace_id: &str,
    request_id: &str,
    index: u64,
    over: Option<&Over>,
) -> Vec<(&'static str, Value)> {
    let a = over.map_or_else(|| request_args(request_id), |o| o.args.clone());
    let prior = over.map_or(Value::Null, |o| json!(o.prior_sha256));
    let (path, content) = (a["path"].as_str().unwrap(), a["content"].as_str().unwrap());
    let base = 20 + 10 * index;
    let (claim, evidence, promotion, committed, grant, intent, ack) = (
        base,
        base + 4,
        base + 5,
        base + 6,
        base + 7,
        base + 8,
        base + 9,
    );
    let content_sha = sha256_hex(content.as_bytes());
    let approved_sha = sha256_hex(&approved_proposal_bytes(path, content));
    let compose_sha = sha256_hex(&compose_proposal_bytes(path, content));
    let approval_id = format!("aien-approval:{request_id}:synthetic");
    let desk = "0123456789abcdef";
    let key = sha256_hex(&approval_binding_bytes(&ApprovalFields {
        approval_id: &approval_id,
        approved_proposal_sha256: &approved_sha,
        approver: "drake",
        content_sha256: &content_sha,
        desk_key_id: desk,
        path,
        request_id,
        trace_id,
        workspace: WORKSPACE,
    }));
    let target = format!("{WORKSPACE}/{path}");
    vec![
        (
            "ledger_claim",
            record_view(
                claim,
                [0; 4],
                "effect",
                &json!({
            "approved_submission": "accepted", "approval_key": key, "request_id": request_id,
            "approval_id": approval_id, "trace_id": trace_id,
            "executor": {"pid": DAEMON_PID, "start": DAEMON_START_TICKS}}),
            ),
        ),
        (
            "ledger_committed",
            record_view(
                committed,
                [claim, 0, 0, 0],
                "effect",
                &json!({
            "approved_submission": "committed", "claim": claim,
            "evidence": {"compose_proposal_sha256": compose_sha, "cx_promotion": promotion,
                         "cx_evidence": evidence, "task": 1}}),
            ),
        ),
        (
            "ledger_grant",
            record_view(
                grant,
                [promotion, evidence, claim, 0],
                "authorization",
                &json!({
            "approved_grant": 1, "proposal_sha256": compose_sha, "path": path,
            "content_sha256": content_sha, "approver": "drake", "target": target,
            "workspace": WORKSPACE, "prior_sha256": prior, "approval_key": key,
            "replay_claim": claim, "cx_promotion": promotion, "cx_evidence": evidence,
            "request_id": request_id, "trace_id": trace_id, "approval_id": approval_id,
            "desk_key_id": desk, "approved_proposal_sha256": approved_sha}),
            ),
        ),
        (
            "ledger_intent",
            record_view(
                intent,
                [grant, 0, 0, 0],
                "effect",
                &json!({
            "phase": "intent", "tool": "write_file", "authorization": grant,
            "proposal_sha256": compose_sha, "path": path, "target": target,
            "content_sha256": content_sha, "prior_sha256": prior,
            "executor": {"pid": 9, "start": 9}}),
            ),
        ),
        (
            "ledger_ack",
            record_view(
                ack,
                [intent, grant, 0, 0],
                "effect",
                &json!({
            "phase": "ack", "intent": intent, "authorization": grant, "tool": "write_file",
            "path": path, "content_sha256": content_sha, "state": "DONE",
            "disk_sha256": content_sha, "disk_error": null}),
            ),
        ),
    ]
}

/// The adapter's receipt as it appears in the INTERPLANE result (`data.receipt`).
fn adapter_receipt(trace_id: &str, request_id: &str, index: u64, over: Option<&Over>) -> Value {
    let s = ledger_slice_over(trace_id, request_id, index, over);
    let r = |n: &str| {
        s.iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| v)
            .expect("record")
    };
    json!({"state": "DONE", "trace_id": trace_id, "request_id": request_id,
        "grant_id": r("ledger_grant")["id"], "grant_digest": r("ledger_grant")["digest"],
        "intent_id": r("ledger_intent")["id"], "intent_digest": r("ledger_intent")["digest"],
        "ack_record_id": r("ledger_ack")["id"], "ack_digest": r("ledger_ack")["digest"]})
}

fn envelope(
    trace_id: &str,
    n: u32,
    parent: Option<String>,
    src: (&str, &str),
    dst: (&str, &str),
    payload: Value,
) -> Value {
    json!({
        "interplane_version": "0.1", "message_id": format!("m-{n}"), "trace_id": trace_id,
        "parent_id": parent, "timestamp": format!("2026-10-06T00:00:{n:02}Z"),
        "source": {"kind": src.0, "id": src.1}, "destination": {"kind": dst.0, "id": dst.1},
        "payload": payload
    })
}

fn args_of(rid: &str, over: Option<&Over>) -> Value {
    match over.filter(|_| rid == REQUEST_ID) {
        Some(o) => o.args.clone(),
        None => request_args(rid),
    }
}

/// Three requests in one trace: 01, 02 and 03, where 03 repeats 01 exactly. In ledger mode each
/// result carries the adapter receipt of its own request.
fn trace(trace_id: &str, weak: bool, over: Option<&Over>) -> Value {
    let mut envs = Vec::new();
    let mut n = 0;
    for (i, rid) in [REQUEST_ID, OTHER_REQUEST_ID, IDENTICAL_REQUEST_ID]
        .into_iter()
        .enumerate()
    {
        n += 1;
        let req = n;
        envs.push(envelope(
            trace_id,
            n,
            None,
            ("model", "synthetic-notes"),
            ("runtime", "aien"),
            json!({
                "kind": "tool_request", "request_id": rid,
                "tool": {"namespace": null, "name": "write_file"},
                "arguments": args_of(rid, over),
                "provenance": {"dialect": "openai", "parser_version": "1"}
            }),
        ));
        n += 1;
        let mut data = match over.filter(|_| rid == REQUEST_ID) {
            Some(o) => {
                json!({"path": o.args["path"], "bytes_written": o.args["content"].as_str().unwrap_or("").len()})
            }
            None => result_data(rid),
        };
        if !weak {
            data["receipt"] =
                adapter_receipt(trace_id, rid, i as u64, over.filter(|_| rid == REQUEST_ID));
        }
        envs.push(envelope(trace_id, n, Some(format!("m-{req}")), ("runtime", "aien"), ("model", "synthetic-notes"), json!({
            "kind": "result", "request_id": rid, "status": "ok", "data": data,
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
        "checkpoint loaded from /models/synthetic-notes/model.safetensors (tokenizer loaded, model_id=synthetic-notes, config=synthetic-notes (config.json), model_sha256={}, tokenizer_sha256={})\n\u{2713} Binding socket at /synthetic/run/aien.sock\n",
        sha256_hex(&exp_w), sha256_hex(&tokenizer)
    );
    let trace_id = o.trace_id.clone().unwrap_or_else(|| TRACE_ID.to_string());
    let request_id = o
        .request_id
        .clone()
        .unwrap_or_else(|| REQUEST_ID.to_string());
    let index = [REQUEST_ID, OTHER_REQUEST_ID, IDENTICAL_REQUEST_ID]
        .iter()
        .position(|r| *r == request_id)
        .expect("request id is one of the synthetic requests") as u64;
    let over = fix_over(o);
    let trace = pretty(&trace(&trace_id, o.weak, over.as_ref()));

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
    ];
    let mut files = files;
    if o.weak {
        files.push((
            "effect_receipt",
            "records/aien/effect-receipt.json",
            receipt_bytes(
                "write_file",
                &request_args(&request_id),
                &result_data(&request_id),
                true,
            ),
        ));
    } else {
        for (name, view) in ledger_slice_over(&trace_id, &request_id, index, over.as_ref()) {
            files.push((name, ledger_path(name), pretty(&view)));
        }
        files.push((
            "daemon_run",
            "records/aien/daemon-run.json",
            pretty(&json!({"kind": "aien-daemon-run", "pid": DAEMON_PID,
                "start_ticks": DAEMON_START_TICKS, "boot_id": "synthetic-boot",
                "socket": "/synthetic/run/aien.sock", "workspace": WORKSPACE,
                "executable_sha256": exe_sha,
                "note": "SYNTHETIC: written by the generator, not by a run harness"})),
        ));
    }
    let fix = o.fix_the_test.then(|| fix_the_test(o));
    if let Some((extra_files, _)) = &fix {
        for (n, p, b) in extra_files {
            files.push((n, p, b.clone()));
        }
    }
    let mut records = serde_json::Map::new();
    for (name, rel, bytes) in &files {
        put(dir, rel, bytes)?;
        records.insert(name.to_string(), json!({"path": rel}));
    }
    let mut skeleton = json!({
        "kind": crate::KIND, "schema": crate::SCHEMA,
        "fixture": {"class": "synthetic", "generator": "provenance/src/synth.rs",
            "note": "Every byte is synthetic. The export carries a Hugging Face tokenizer.json, which a real WALDO Hugging Face export does not; no AIEN process loaded this model and no runtime wrote this receipt."},
        "completeness": {"state": "incomplete", "missing": ["link:model_turn"],
            "reason": "the INTERPLANE model turn is synthetic: nothing links this model to the effect's content"},
        "records": records,
        "lineage": {"kind": "waldo_run", "run_id": RUN_ID, "model_id": model_id,
            "corpus_license_assertions": ["CC0-1.0"], "training_backend": backend,
            "training_code": {"repository": "github.com/openwaldo/waldo", "commit": "synthetic", "checked": false}},
        "export": {"format": "huggingface", "model_revision": format!("{model_id}/{RUN_ID}"),
            "chat_template": "export_chat_template",
            "conversion_tool": {"name": "waldo model export --format huggingface", "commit": "synthetic", "checked": false}},
        "load_support": {"declared": crate::SUPPORTED},
        "aien": {"candidate_id": CAND, "executable": EXE, "executable_sha256": exe_sha},
        "interplane": {"trace_id": trace_id, "request_id": request_id},
        "effect": if o.weak {
            json!({"binding": RECEIPT_BINDING, "digest_form": SERDE_DIGEST_FORM, "receipt": "effect_receipt"})
        } else {
            json!({"binding": LEDGER_BINDING, "approval_binding": APPROVAL_BINDING, "proposal_origin": "scripted_turn"})
        }
    });
    if let Some((_, extra)) = fix {
        skeleton["test_run"] = extra["test_run"].clone();
        let mut missing = vec!["link:model_turn"];
        match o.native {
            Some(true) => {}
            _ => missing.push("link:native"),
        }
        if !extra["aien_native"].is_null() {
            skeleton["aien"]["native"] = extra["aien_native"].clone();
        }
        skeleton["completeness"]["missing"] = json!(missing);
    }
    seal(dir, skeleton)
}

fn ledger_path(name: &str) -> &'static str {
    match name {
        "ledger_claim" => "records/aien/ledger-claim.json",
        "ledger_committed" => "records/aien/ledger-committed.json",
        "ledger_grant" => "records/aien/ledger-grant.json",
        "ledger_intent" => "records/aien/ledger-intent.json",
        _ => "records/aien/ledger-ack.json",
    }
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

/// The SYNTHETIC buggy and fixed `src/clamp.c` of the fix-the-test slice (a stand-in for the
/// adapter fixture; the verifier never reads either, only their digests).
pub const BUGGY_C: &str = "int clamp(int v, int lo, int hi)\n{\n    if (v < lo)\n        return lo;\n    if (v > hi)\n        return lo;\n    return v;\n}\n";
pub const FIXED_C: &str = "int clamp(int v, int lo, int hi)\n{\n    if (v < lo)\n        return lo;\n    if (v > hi)\n        return hi;\n    return v;\n}\n";
pub const TASK_ID: &str = "fix-the-test/clamp-1";
pub const SOURCE_COMMIT: &str = "1111111111111111111111111111111111111111";
pub const TREE_AFTER: &str = "2222222222222222222222222222222222222222";
pub const OMEGA_SHA: &str = "6c6180cf378075b61291f4565d226eba38b4decd";
pub const TEST_STDOUT: &str = "ok   inside\nok   below\nok   above\nok   at_low\nok   at_high\n";

fn fix_over(o: &Opts) -> Option<Over> {
    o.fix_the_test.then(|| Over {
        args: json!({"path": "src/clamp.c", "content": FIXED_C}),
        prior_sha256: sha256_hex(BUGGY_C.as_bytes()),
    })
}

/// Extra records and companion sections of the fix-the-test slice, merged into the skeleton by
/// `write_synthetic`: `(name, path, bytes)` files, and companion keys to set.
type ExtraFile = (&'static str, &'static str, Vec<u8>);

fn fix_the_test(o: &Opts) -> (Vec<ExtraFile>, Value) {
    let task = pretty(
        &json!({"kind": "vac-task", "task_id": TASK_ID, "repo_commit": SOURCE_COMMIT,
        "test_cmd": ["make", "test"], "target_path": "src/clamp.c"}),
    );
    let pin = pretty(
        &json!({"kind": "vac-source-pin", "repo": "fix_the_test", "commit": SOURCE_COMMIT,
        "target_path": "src/clamp.c", "target_blob_sha256": sha256_hex(BUGGY_C.as_bytes())}),
    );
    let stdout = TEST_STDOUT.as_bytes().to_vec();
    let stderr = Vec::new();
    let run = pretty(&json!({"kind": "vac-test-run", "v": 1, "task_id": TASK_ID,
        "argv": ["make", "test"], "cwd_rel": ".", "exit_code": 0, "test_exit_before": 2,
        "stdout_sha256": sha256_hex(&stdout), "stderr_sha256": sha256_hex(&stderr),
        "started_unix_ms": 1_791_000_000_000u64, "duration_ms": 120,
        "tree_commit_after": TREE_AFTER, "target_blob_sha256_after": sha256_hex(FIXED_C.as_bytes())}));
    let mut files = vec![
        ("task", "records/task/task.json", task),
        ("source_pin", "records/task/source-pin.json", pin),
        ("test_run_record", "records/task/test-run.json", run),
        ("test_stdout", "records/task/test-stdout.txt", stdout),
        ("test_stderr", "records/task/test-stderr.txt", stderr),
    ];
    let mut extra = json!({"test_run": {"binding": "vac-test-run/1", "task_id": TASK_ID,
        "record": "test_run_record"}});
    if let Some(claimed) = o.native {
        let recall = pretty(
            &json!({"compose_dir": "/synthetic/compose", "machine_id": "synthetic",
            "records_total": 30, "host": [], "cited": [], "missing": [],
            "compose_native": claimed, "omega_sha": if claimed { OMEGA_SHA } else { "" }}),
        );
        files.push(("compose_recall", "records/aien/compose-recall.json", recall));
        extra["aien_native"] =
            json!({"claimed": claimed, "omega_sha": if claimed { OMEGA_SHA } else { "" }});
    }
    (files, extra)
}
