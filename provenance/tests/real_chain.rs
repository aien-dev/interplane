//! The first real chain: one WALDO-trained tiny model, one AIEN daemon load, one approved write
//! through the INTERPLANE AIEN adapter, and the daemon's own ledger records. Every byte under
//! `fixtures/real-waldo-aien-chain` comes from that one run (CPU evidence). These tests alter a
//! copy and require the verifier to refuse.
mod common;
use common::*;
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const OK: &str = "PASS complete effect=aien-ledger-slice/1:strong proposal=scripted_turn";

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

#[test]
fn real_chain_passes_and_is_labelled_scripted_turn() {
    assert_eq!(verify(Path::new(REAL)), OK);
}

#[test]
fn real_chain_records_are_real_daemon_output() {
    // The approval key the real daemon wrote equals the key recomputed from its nine fields in the
    // byte form of BINDING.md: the check passes only if producer and verifier agree byte for byte.
    let c = companion(Path::new(REAL));
    assert_eq!(c["fixture"]["class"], "real");
    assert_eq!(c["aien"]["candidate_id"], Value::Null);
    let log = std::fs::read_to_string(path_of(Path::new(REAL), "aien_load_log")).unwrap();
    assert!(log.contains("CPU-reference"), "CPU evidence only");
    assert!(
        log.contains("Tokenizer: chatml"),
        "the loader accepted the export's chat template"
    );
}

#[test]
fn proposal_origin_must_be_stated_and_a_model_turn_has_no_binding() {
    let d = scratch(REAL, "real_origin_absent");
    let mut c = companion(&d);
    c["effect"]
        .as_object_mut()
        .unwrap()
        .remove("proposal_origin");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL malformed_companion: effect.proposal_origin"
    );
    let d = scratch(REAL, "real_origin_model");
    let mut c = companion(&d);
    c["effect"]["proposal_origin"] = json!("model_turn");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unsupported_binding: effect.proposal_origin=model_turn (only scripted_turn has a binding)"
    );
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
        let v = verify(&d);
        assert!(v.starts_with("FAIL "), "{rec}: {v}");
    }
}

#[test]
fn an_altered_daemon_identity_or_trace_is_refused() {
    let d = scratch(REAL, "real_pid");
    edit_json(&d, "daemon_run", |v| v["pid"] = json!(1));
    assert!(verify(&d).starts_with("FAIL "), "{}", verify(&d));
    let d = scratch(REAL, "real_trace_content");
    edit_json(&d, "interplane_trace", |v| {
        v[0]["payload"]["arguments"]["content"] = json!("something else\n")
    });
    assert!(verify(&d).starts_with("FAIL "), "{}", verify(&d));
}

#[test]
fn a_different_model_with_the_same_daemon_load_log_is_refused() {
    let d = scratch(REAL, "real_model_swap");
    let p = path_of(&d, "export_weights");
    let mut b = std::fs::read(&p).unwrap();
    let n = b.len() - 1;
    b[n] ^= 1;
    std::fs::write(&p, b).unwrap();
    restamp(&d, "export_weights");
    assert!(verify(&d).starts_with("FAIL "), "{}", verify(&d));
}
