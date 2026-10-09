//! `model-turn/2`: the retained model turn names the exact request and response bytes, the model
//! and weights digests, the backend and the times, and each is compared with what the bundle
//! retains. The base is the real daemon-generated chain (`fixtures/real-waldo-aien-chain`) upgraded
//! in a copy with the new request record and fields, so the committed fixture is never rewritten.
//! Every expected verdict is a literal. A turn that does not chain never yields `PASS`: it is
//! `FAIL`, or, when the bundle honestly says the turn is not a model's, the INCOMPLETE label.
mod common;
use common::*;
use interplane_provenance::gojson::sha256_hex;
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const COMPLETE: &str =
    "PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2 candidate=none";
const REQUEST: &[u8] =
    b"[{\"role\":\"user\",\"content\":\"Save the meeting summary to summary.txt.\"}]";

fn edit_json(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

/// A copy of the real chain whose model turn is a `model-turn/2` record with a retained request.
fn v2(name: &str) -> std::path::PathBuf {
    let d = scratch(REAL, name);
    std::fs::create_dir_all(d.join("records/model")).unwrap();
    std::fs::write(d.join("records/model/request.json"), REQUEST).unwrap();
    let mut c = companion(&d);
    c["records"]["model_request"] = json!({"path": "records/model/request.json",
        "sha256": sha256_hex(REQUEST), "bytes": REQUEST.len()});
    let weights = c["records"]["export_weights"]["sha256"].clone();
    write_companion(&d, &c);
    let log = std::fs::read_to_string(path_of(&d, "aien_load_log")).unwrap();
    let backend = log
        .lines()
        .find(|l| l.contains("Backend:"))
        .unwrap()
        .trim()
        .to_string();
    edit_json(&d, "model_turn", |t| {
        let input = t["input"].as_str().unwrap().to_string();
        t["record"] = json!("model-turn/2");
        t["model_id"] = json!("toolcall-tiny");
        t["request_record"] = json!("model_request");
        t["request_sha256"] = json!(sha256_hex(REQUEST));
        t["response_sha256"] = json!(sha256_hex(input.as_bytes()));
        t["weights_sha256"] = weights;
        t["backend"] = json!(backend);
    });
    d
}

fn failed(d: &Path, code: &str, why: &str) {
    let v = verify(d);
    assert!(
        v.starts_with(&format!("FAIL {code}:")) && !v.starts_with("PASS"),
        "{why}: {v}"
    );
}

#[test]
fn a_model_turn_that_chains_by_digest_is_complete() {
    let d = v2("mt-ok");
    assert_eq!(verify(&d), COMPLETE);
}

#[test]
fn request_bytes_changed_after_the_digest_was_named_are_refused() {
    let d = v2("mt-req");
    std::fs::write(
        d.join("records/model/request.json"),
        b"[{\"role\":\"user\",\"content\":\"other\"}]",
    )
    .unwrap();
    restamp(&d, "model_request");
    failed(
        &d,
        "binding_mismatch",
        "request bytes differ from request_sha256",
    );
}

#[test]
fn a_request_that_is_not_a_messages_array_is_refused() {
    let d = v2("mt-req-shape");
    std::fs::write(d.join("records/model/request.json"), b"{}").unwrap();
    restamp(&d, "model_request");
    edit_json(&d, "model_turn", |t| {
        t["request_sha256"] = json!(sha256_hex(b"{}"))
    });
    failed(&d, "binding_mismatch", "request is not a messages array");
}

#[test]
fn a_request_record_that_is_not_listed_is_refused() {
    let d = v2("mt-req-unlisted");
    let mut c = companion(&d);
    c["records"]
        .as_object_mut()
        .unwrap()
        .remove("model_request");
    write_companion(&d, &c);
    failed(
        &d,
        "missing_record_entry",
        "request record dropped from the manifest",
    );
}

#[test]
fn a_response_that_is_not_the_named_response_is_refused() {
    let d = v2("mt-resp");
    edit_json(&d, "model_turn", |t| {
        t["response_sha256"] = json!(sha256_hex(b"other"))
    });
    failed(&d, "binding_mismatch", "response_sha256 differs");
}

#[test]
fn weights_digest_not_the_exported_file_is_refused() {
    let d = v2("mt-weights");
    edit_json(&d, "model_turn", |t| {
        t["weights_sha256"] = json!(sha256_hex(b"other weights"))
    });
    failed(&d, "binding_mismatch", "weights_sha256 differs");
}

#[test]
fn a_backend_the_load_log_does_not_name_is_refused() {
    let d = v2("mt-backend");
    edit_json(&d, "model_turn", |t| t["backend"] = json!("Backend: CUDA"));
    failed(&d, "binding_mismatch", "backend not in the log");
}

#[test]
fn times_out_of_order_or_after_the_request_are_refused() {
    let d = v2("mt-time-order");
    edit_json(&d, "model_turn", |t| {
        t["stream_started"] = json!("2026-10-08T02:00:00Z");
        t["stream_finished"] = json!("2026-10-08T01:00:00Z");
    });
    failed(&d, "binding_mismatch", "finished before started");
    let d = v2("mt-time-late");
    edit_json(&d, "model_turn", |t| {
        t["stream_started"] = json!("2026-10-08T03:00:00Z");
        t["stream_finished"] = json!("2026-10-08T03:00:01Z");
    });
    failed(
        &d,
        "binding_mismatch",
        "generation finished after the request",
    );
}

#[test]
fn a_turn_for_other_bytes_does_not_chain_to_the_effect() {
    // The response text is replaced by a different tool call and every digest is restamped to
    // match: the daemon's generation record and the trace request still name the old text.
    let d = v2("mt-other-text");
    edit_json(&d, "model_turn", |t| {
        let other = "<tool_call>\n{\"name\": \"write_file\", \"arguments\": {\"path\": \"summary.txt\", \"content\": \"other\\n\"}}\n</tool_call><|im_end|>\n";
        t["input"] = json!(other);
        t["response_sha256"] = json!(sha256_hex(other.as_bytes()));
    });
    failed(
        &d,
        "binding_mismatch",
        "text differs from the daemon's generation record",
    );
}

#[test]
fn a_missing_model_turn_is_a_refusal_and_a_scripted_one_is_labelled_incomplete() {
    // Declared as a model's, not retained: refused.
    let d = v2("mt-dropped");
    let mut c = companion(&d);
    c["records"].as_object_mut().unwrap().remove("model_turn");
    write_companion(&d, &c);
    assert!(verify(&d).starts_with("FAIL "), "{}", verify(&d));
    // The honest downgrade: the same bundle says the turn was scripted. Never PASS complete.
    let d = v2("mt-scripted");
    let mut c = companion(&d);
    c["effect"]["proposal_origin"] = json!("scripted_turn");
    c["completeness"] = json!({"state": "incomplete", "missing": ["link:model_turn"]});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
    );
    // A scripted turn that claims completeness is refused.
    let mut c = companion(&d);
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    failed(&d, "unlabelled_missing", "scripted but declared complete");
}

#[test]
fn the_version_one_turn_keeps_its_old_checks_and_is_not_read_as_having_the_new_links() {
    let d = scratch(REAL, "mt-v1");
    assert_eq!(verify(&d), COMPLETE);
    let t: Value =
        serde_json::from_slice(&std::fs::read(path_of(&d, "model_turn")).unwrap()).unwrap();
    assert!(t.get("record").is_none() && t.get("request_sha256").is_none());
}
