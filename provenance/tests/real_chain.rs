//! The first real chain: one WALDO-trained tiny model, one AIEN daemon load, a tool call generated
//! by AIEN's own generation path on that loaded model, parsed by the INTERPLANE `aien_legacy`
//! dialect, one approved write through the INTERPLANE AIEN adapter, and the daemon's own ledger
//! records, including the daemon's OWN record of the generation. Every byte under `fixtures/real-waldo-aien-chain` comes from that one run (CPU
//! evidence). These tests alter a copy and require the verifier to refuse.
mod common;
use common::*;
use interplane_provenance::gojson::sha256_hex;
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const OK: &str = "PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2";

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
fn real_chain_is_complete_because_every_link_is_verified_from_retained_bytes() {
    assert_eq!(verify(Path::new(REAL)), OK);
    let c = companion(Path::new(REAL));
    assert_eq!(c["completeness"]["state"], "complete");
    assert_eq!(c["completeness"]["missing"], json!([]));
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
    // The generation record is the daemon's: an `effect` note carrying the marker, written before
    // the approved write, and the id the daemon returned in TurnFinished is the retained one.
    let r: Value = serde_json::from_slice(
        &std::fs::read(path_of(Path::new(REAL), "ledger_generation")).unwrap(),
    )
    .unwrap();
    let g: Value = serde_json::from_str(r["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        (g["generation"].clone(), g["v"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(r["note"], "effect");
    assert_eq!(r["verified"], true);
    let t: Value =
        serde_json::from_slice(&std::fs::read(path_of(Path::new(REAL), "model_turn")).unwrap())
            .unwrap();
    assert_eq!(t["generation_record_id"], r["id"]);
    assert_eq!(
        t["generation_record_source"],
        "TurnFinished.generation_record"
    );
}

#[test]
fn a_scripted_turn_is_never_complete() {
    // Same real bytes, but declared as a scripted turn: the model link is missing.
    let d = scratch(REAL, "real_scripted");
    let mut c = companion(&d);
    c["effect"]["proposal_origin"] = json!("scripted_turn");
    c["completeness"] =
        json!({"state": "incomplete", "missing": ["link:model_turn"], "reason": "scripted"});
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
fn the_driver_asserted_generation_origin_is_superseded() {
    let d = scratch(REAL, "real_origin_v1");
    let mut c = companion(&d);
    c["effect"]["proposal_origin"] = json!("model_generation/1");
    write_companion(&d, &c);
    let v = verify(&d);
    assert!(
        v.starts_with("FAIL unsupported_binding: effect.proposal_origin=model_generation/1"),
        "{v}"
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
fn the_generated_text_must_be_the_text_the_dialect_parsed() {
    // The model turn is changed to a different but valid call; the daemon's record still carries
    // the digest of what the model returned.
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
        "FAIL binding_mismatch: generation record output_text_sha256 != sha256(model_turn.input)"
    );
}

#[test]
fn a_consistently_rewritten_turn_and_record_still_has_to_match_the_trace_request() {
    let d = scratch(REAL, "real_turn_rewrite");
    let new = |s: &str| s.replace("summary.txt", "other.txt");
    edit_json(&d, "model_turn", |t| {
        t["input"] = json!(new(t["input"].as_str().unwrap()))
    });
    let text = |d: &Path| {
        serde_json::from_slice::<Value>(&std::fs::read(path_of(d, "model_turn")).unwrap()).unwrap()
            ["input"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let digest = sha256_hex(text(&d).as_bytes());
    edit_text(&d, "ledger_generation", |g| {
        g["output_text_sha256"] = json!(digest)
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: model_turn arguments vs trace request"
    );
}

#[test]
fn a_record_for_another_model_or_tokenizer_is_refused() {
    let d = scratch(REAL, "real_gen_model");
    edit_text(&d, "ledger_generation", |g| {
        g["model_sha256"] = json!("1".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation record model_sha256 != export_weights"
    );
    let d = scratch(REAL, "real_gen_tokenizer");
    edit_text(&d, "ledger_generation", |g| {
        g["tokenizer_sha256"] = json!("2".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation record tokenizer_sha256 != export_tokenizer"
    );
}

#[test]
fn a_record_from_another_daemon_is_refused() {
    let d = scratch(REAL, "real_gen_pid");
    edit_text(&d, "ledger_generation", |g| g["daemon"]["pid"] = json!(1));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation record daemon pid/start vs daemon_run"
    );
    let d = scratch(REAL, "real_gen_start");
    edit_text(&d, "ledger_generation", |g| {
        g["daemon"]["start_ticks"] = json!(1)
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation record daemon pid/start vs daemon_run"
    );
}

#[test]
fn a_missing_generation_record_is_an_absent_claim_not_a_pass() {
    // Not retained at all: the manifest has no entry.
    let d = scratch(REAL, "real_gen_missing");
    let mut c = companion(&d);
    c["records"]
        .as_object_mut()
        .unwrap()
        .remove("ledger_generation");
    write_companion(&d, &c);
    assert_eq!(verify(&d), "FAIL missing_record_entry: ledger_generation");
    // Listed but the bytes are gone.
    let d = scratch(REAL, "real_gen_deleted");
    std::fs::remove_file(path_of(&d, "ledger_generation")).unwrap();
    let v = verify(&d);
    assert!(v.starts_with("FAIL "), "{v}");
    // The daemon returned no id: the run cannot claim a record.
    let d = scratch(REAL, "real_gen_no_id");
    edit_json(&d, "model_turn", |t| {
        t.as_object_mut().unwrap().remove("generation_record_id");
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: model_turn.generation_record_id != ledger_generation.id"
    );
}

#[test]
fn a_generation_record_that_is_not_the_one_the_daemon_returned_is_refused() {
    let d = scratch(REAL, "real_gen_other_id");
    edit_json(&d, "ledger_generation", |r| r["id"] = json!(3));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: model_turn.generation_record_id != ledger_generation.id"
    );
}

#[test]
fn a_generation_record_must_precede_the_effect() {
    let d = scratch(REAL, "real_gen_late");
    edit_json(&d, "ledger_generation", |r| r["id"] = json!(99));
    edit_json(&d, "model_turn", |t| t["generation_record_id"] = json!(99));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: generation record does not precede the effect's replay claim"
    );
}

#[test]
fn a_malformed_generation_record_is_refused() {
    type Case = (&'static str, fn(&mut Value), &'static str);
    let cases: [Case; 6] = [
        (
            "unverified",
            |r| r["verified"] = json!(false),
            "ledger_generation.verified",
        ),
        (
            "wrong_note",
            |r| r["note"] = json!("observation"),
            "ledger_generation.note",
        ),
        (
            "links",
            |r| r["links"] = json!([1, 0, 0, 0]),
            "ledger_generation.links (must be empty)",
        ),
        (
            "not_json",
            |r| r["text"] = json!("generation"),
            "ledger_generation.text (not one JSON object)",
        ),
        (
            "trailing",
            |r| {
                let t = format!("{} {{}}", r["text"].as_str().unwrap());
                r["text"] = json!(t)
            },
            "ledger_generation.text (not one JSON object)",
        ),
        (
            "array",
            |r| r["text"] = json!("[1]"),
            "ledger_generation.text (not one JSON object)",
        ),
    ];
    for (name, f, msg) in cases {
        let d = scratch(REAL, &format!("real_gen_bad_{name}"));
        edit_json(&d, "ledger_generation", f);
        assert_eq!(
            verify(&d),
            format!("FAIL binding_mismatch: {msg}"),
            "{name}"
        );
    }
    type Edit = fn(&mut Value);
    let text_cases: [(&str, Edit, &str); 6] = [
        (
            "marker",
            |g| g["generation"] = json!(2),
            "ledger_generation.generation/v marker",
        ),
        (
            "marker_absent",
            |g| {
                g.as_object_mut().unwrap().remove("generation");
            },
            "ledger_generation.generation/v marker",
        ),
        (
            "version",
            |g| g["v"] = json!(2),
            "ledger_generation.generation/v marker",
        ),
        (
            "aborted",
            |g| g["finish_reason"] = json!("aborted"),
            "generation record finish_reason=aborted",
        ),
        (
            "no_tokens",
            |g| g["output_tokens"] = json!(0),
            "generation record output_tokens",
        ),
        (
            "ids_digest",
            |g| g["prompt_ids_sha256"] = json!("zz"),
            "generation record prompt_ids_sha256",
        ),
    ];
    for (name, f, msg) in text_cases {
        let d = scratch(REAL, &format!("real_gen_badtext_{name}"));
        edit_text(&d, "ledger_generation", f);
        assert_eq!(
            verify(&d),
            format!("FAIL binding_mismatch: {msg}"),
            "{name}"
        );
    }
}

#[test]
fn an_altered_generation_record_is_refused_whatever_field_changes() {
    for (k, v) in [
        ("output_text_sha256", json!("3".repeat(64))),
        ("model_sha256", json!("4".repeat(64))),
        ("tokenizer_sha256", json!("5".repeat(64))),
        ("finish_reason", json!("preempted")),
    ] {
        let d = scratch(REAL, &format!("real_gen_alter_{k}"));
        edit_text(&d, "ledger_generation", |g| g[k] = v.clone());
        refused(&d, k);
    }
    // The caller-asserted ids are recorded, never trusted: changing them changes no verdict.
    let d = scratch(REAL, "real_gen_claimed_ids");
    edit_text(&d, "ledger_generation", |g| g["request_id"] = json!(1));
    assert_eq!(verify(&d), OK);
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
