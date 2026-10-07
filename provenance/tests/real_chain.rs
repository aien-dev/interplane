//! The first real chain: one WALDO-trained tiny model, one AIEN daemon load, a tool call generated
//! by AIEN's own generation path on that loaded model, parsed by the INTERPLANE `aien_legacy`
//! dialect, one approved write through the INTERPLANE AIEN adapter, and the daemon's own ledger
//! records. Every byte under `fixtures/real-waldo-aien-chain` comes from that one run (CPU
//! evidence). These tests alter a copy and require the verifier to refuse.
mod common;
use common::*;
use interplane_provenance::gojson::sha256_hex;
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const OK: &str = "PASS_LABELLED_INCOMPLETE missing=link:daemon_generation_record effect=aien-ledger-slice/1:strong proposal=model_generation/1";

fn edit_json(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

/// Edit the JSON carried inside a ComposeRecordView's `text` string.
fn edit_text(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    edit_json(d, rec, |v| {
        let mut t: Value = serde_json::from_str(v["text"].as_str().unwrap()).unwrap();
        f(&mut t);
        v["text"] = json!(serde_json::to_string(&t).unwrap());
    });
}

fn refused(d: &Path, why: &str) {
    let v = verify(d);
    assert!(v.starts_with("FAIL "), "{why}: {v}");
}

#[test]
fn real_chain_is_labelled_incomplete_never_complete() {
    assert_eq!(verify(Path::new(REAL)), OK);
    let c = companion(Path::new(REAL));
    assert_eq!(c["completeness"]["state"], "incomplete");
    assert_eq!(
        c["completeness"]["missing"],
        json!(["link:daemon_generation_record"])
    );
}

#[test]
fn real_chain_records_are_real_daemon_output() {
    let c = companion(Path::new(REAL));
    assert_eq!(c["fixture"]["class"], "real");
    assert_eq!(c["aien"]["candidate_id"], Value::Null);
    let log = std::fs::read_to_string(path_of(Path::new(REAL), "aien_load_log")).unwrap();
    assert!(log.contains("CPU-reference"), "CPU evidence only");
    assert!(
        log.contains("Tokenizer: chatml"),
        "the loader accepted the export's chat template"
    );
    assert!(
        log.contains("Warm-up: 1 token in"),
        "the loaded model really ran inference"
    );
    // The approval key the real daemon wrote equals the key recomputed from its nine fields
    // (checked by the verifier above): producer and verifier agree byte for byte on real output.
    let g: Value =
        serde_json::from_slice(&std::fs::read(path_of(Path::new(REAL), "generation")).unwrap())
            .unwrap();
    assert_eq!(g["written_by"], "run-driver");
    assert_eq!(
        g["journal_records_before"], g["journal_records_after"],
        "no daemon record of the generation"
    );
}

#[test]
fn a_scripted_turn_is_never_complete() {
    // Same real bytes, but declared as a scripted turn: the model link is missing.
    let d = scratch(REAL, "real_scripted");
    let mut c = companion(&d);
    c["effect"]["proposal_origin"] = json!("scripted_turn");
    c["completeness"]["missing"] = json!(["link:model_turn"]);
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "PASS_LABELLED_INCOMPLETE missing=link:model_turn effect=aien-ledger-slice/1:strong proposal=scripted_turn"
    );
    // Declaring it complete is refused, whatever the bytes say.
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unlabelled_missing: link:model_turn (declared complete)"
    );
}

#[test]
fn declaring_the_real_chain_complete_is_refused() {
    let d = scratch(REAL, "real_declared_complete");
    let mut c = companion(&d);
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unlabelled_missing: link:daemon_generation_record (declared complete)"
    );
}

#[test]
fn proposal_origin_must_be_stated_and_unknown_origins_are_refused() {
    let d = scratch(REAL, "real_origin_absent");
    let mut c = companion(&d);
    c["effect"]
        .as_object_mut()
        .unwrap()
        .remove("proposal_origin");
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL malformed_companion: effect.proposal_origin"
    );
    c["effect"]["proposal_origin"] = json!("model_turn");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unsupported_binding: effect.proposal_origin=model_turn"
    );
}

#[test]
fn a_daemon_written_generation_record_has_no_binding_yet() {
    let d = scratch(REAL, "real_written_by_daemon");
    edit_json(&d, "generation", |g| g["written_by"] = json!("daemon"));
    let mut c = companion(&d);
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unsupported_binding: generation.written_by=daemon (no daemon generation record exists to bind)"
    );
}

#[test]
fn the_generated_text_must_be_the_text_the_dialect_parsed() {
    // The model turn is changed to a different but valid call; the generation still says what the
    // model returned.
    let d = scratch(REAL, "real_turn_text");
    edit_json(&d, "model_turn", |t| {
        let s = t["input"]
            .as_str()
            .unwrap()
            .replace("summary.txt", "other.txt");
        t["input"] = json!(s);
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation.output_text != model_turn.input"
    );
}

#[test]
fn a_consistently_rewritten_turn_still_has_to_match_the_trace_request() {
    let d = scratch(REAL, "real_turn_rewrite");
    let new = |s: &str| s.replace("summary.txt", "other.txt");
    edit_json(&d, "model_turn", |t| {
        t["input"] = json!(new(t["input"].as_str().unwrap()))
    });
    edit_json(&d, "generation", |g| {
        let n = new(g["output_text"].as_str().unwrap());
        g["output_sha256"] = json!(sha256_hex(n.as_bytes()));
        g["output_text"] = json!(n);
    });
    let v = verify(&d);
    assert_eq!(
        v, "FAIL binding_mismatch: model_turn arguments vs trace request",
        "{v}"
    );
}

#[test]
fn the_generation_digest_model_and_process_are_checked() {
    let d = scratch(REAL, "real_gen_sha");
    edit_json(&d, "generation", |g| {
        g["output_sha256"] = json!("0".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation.output_sha256"
    );
    let d = scratch(REAL, "real_gen_model");
    edit_json(&d, "generation", |g| {
        g["model_sha256_from_load_log"] = json!("1".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation.model_sha256 != export_weights"
    );
    let d = scratch(REAL, "real_gen_pid");
    edit_json(&d, "generation", |g| g["daemon_pid"] = json!(1));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation.daemon_pid/start vs daemon_run"
    );
    let d = scratch(REAL, "real_source");
    edit_json(&d, "interplane_trace", |t| {
        t[0]["source"]["id"] = json!("someone-else")
    });
    refused(&d, "trace source names another model");
}

#[test]
fn an_altered_ledger_record_of_each_type_is_refused() {
    type Edit = fn(&mut Value);
    let cases: [(&str, Edit); 5] = [
        ("ledger_claim", |t| t["request_id"] = json!("w2")),
        ("ledger_committed", |t| t["claim"] = json!(999)),
        ("ledger_grant", |t| t["path"] = json!("other.txt")),
        ("ledger_intent", |t| t["path"] = json!("other.txt")),
        ("ledger_ack", |t| t["state"] = json!("FAILED")),
    ];
    for (rec, f) in cases {
        let d = scratch(REAL, &format!("real_alter_{rec}"));
        edit_text(&d, rec, f);
        refused(&d, rec);
    }
}

#[test]
fn an_altered_daemon_identity_trace_or_weights_is_refused() {
    let d = scratch(REAL, "real_pid");
    edit_json(&d, "daemon_run", |v| v["pid"] = json!(1));
    refused(&d, "daemon pid");
    let d = scratch(REAL, "real_trace_content");
    edit_json(&d, "interplane_trace", |v| {
        v[0]["payload"]["arguments"]["content"] = json!("something else\n")
    });
    refused(&d, "trace content");
    let d = scratch(REAL, "real_model_swap");
    let p = path_of(&d, "export_weights");
    let mut b = std::fs::read(&p).unwrap();
    let n = b.len() - 1;
    b[n] ^= 1;
    std::fs::write(&p, b).unwrap();
    restamp(&d, "export_weights");
    refused(&d, "weights");
}
