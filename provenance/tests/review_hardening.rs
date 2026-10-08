//! #76 independent review findings, each as a refusal on a copy of a committed fixture:
//! duplicate JSON keys (manifest, record, and the JSON inside a ledger note), test material that
//! claims a complete chain, symlinked records, a second final result in the trace, a malformed
//! `retained` flag, a repeated candidate-manifest key, and the candidate on the verdict line.
mod common;
use common::*;
use interplane_provenance::verify;
use serde_json::json;
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const SYN: &str = "fixtures/synthetic-full-chain";

/// Replace the first `from` in the record's raw bytes, then restamp its size and digest.
fn edit_raw(d: &Path, rec: &str, from: &str, to: &str) {
    let p = path_of(d, rec);
    let t = std::fs::read_to_string(&p).unwrap();
    assert!(t.contains(from), "{rec} lacks {from:?}");
    std::fs::write(&p, t.replacen(from, to, 1)).unwrap();
    restamp(d, rec);
}

#[test]
fn the_real_chain_names_its_missing_candidate_on_the_verdict_line() {
    assert_eq!(
        verify(Path::new(env!("CARGO_MANIFEST_DIR")).join(REAL).as_path()),
        "PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2 candidate=none"
    );
}

#[test]
fn a_duplicate_key_in_the_manifest_is_refused() {
    let d = scratch(REAL, "dup_manifest");
    let p = d.join("COMPANION.json");
    let t = std::fs::read_to_string(&p).unwrap();
    let t = t.replacen(
        "\"completeness\": {",
        "\"completeness\": {\"state\": \"incomplete\",",
        1,
    );
    std::fs::write(&p, t).unwrap();
    assert_eq!(verify(&d), "FAIL unparseable_companion: COMPANION.json");
}

#[test]
fn a_duplicate_key_in_a_record_is_refused() {
    let d = scratch(REAL, "dup_trace");
    edit_raw(
        &d,
        "interplane_trace",
        "\"message_id\": \"m-1\",",
        "\"message_id\": \"m-9\", \"message_id\": \"m-1\",",
    );
    assert_eq!(verify(&d), "FAIL unparseable_record: interplane_trace");
}

#[test]
fn a_duplicate_key_inside_a_ledger_note_is_refused() {
    let d = scratch(REAL, "dup_note");
    edit_raw(
        &d,
        "ledger_generation",
        "{\\\"daemon\\\":",
        "{\\\"generation\\\":0,\\\"daemon\\\":",
    );
    let v = verify(&d);
    assert!(v.starts_with("FAIL ") && v.contains("text"), "{v}");
}

#[test]
fn test_material_never_verifies_as_complete() {
    for (class, want) in [
        (
            json!("synthetic"),
            "FAIL synthetic_complete: fixture.class=synthetic",
        ),
        (
            json!("staged"),
            "FAIL synthetic_complete: fixture.class=staged",
        ),
        (json!(null), "FAIL malformed_companion: fixture.class"),
    ] {
        let d = scratch(REAL, "fixture_class");
        let mut c = companion(&d);
        c["fixture"]["class"] = class;
        write_companion(&d, &c);
        assert_eq!(verify(&d), want);
    }
}

#[test]
fn a_symlinked_record_or_directory_is_refused() {
    let d = scratch(REAL, "symlink_file");
    let p = path_of(&d, "export_config");
    let outside = d.with_extension("outside-config.json");
    std::fs::rename(&p, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &p).unwrap();
    assert_eq!(
        verify(&d),
        "FAIL malformed_companion: records.export_config.path (symlink)"
    );

    let d = scratch(REAL, "symlink_dir");
    let dir = d.join("records/export");
    let outside = d.with_extension("outside-export");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::rename(&dir, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &dir).unwrap();
    let v = verify(&d);
    assert!(
        v.starts_with("FAIL malformed_companion: records.export_") && v.ends_with("(symlink)"),
        "{v}"
    );
}

#[test]
fn a_result_after_the_final_one_is_refused() {
    let d = scratch(REAL, "second_final");
    let p = path_of(&d, "interplane_trace");
    let mut t: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let last = t[2].clone();
    assert_eq!(last["payload"]["status"], "ok");
    t.as_array_mut().unwrap().push(last);
    std::fs::write(&p, serde_json::to_vec_pretty(&t).unwrap()).unwrap();
    restamp(&d, "interplane_trace");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: interplane_trace[3] result after the final one for real-run-04:t0:c0"
    );

    let d = scratch(REAL, "second_request");
    let p = path_of(&d, "interplane_trace");
    let mut t: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let req = t[0].clone();
    t.as_array_mut().unwrap().insert(1, req);
    std::fs::write(&p, serde_json::to_vec_pretty(&t).unwrap()).unwrap();
    restamp(&d, "interplane_trace");
    let v = verify(&d);
    assert!(
        v.starts_with("FAIL binding_mismatch: interplane_trace[1] second tool_request"),
        "{v}"
    );
}

#[test]
fn a_non_boolean_retained_flag_is_refused() {
    let d = scratch(REAL, "retained_str");
    let mut c = companion(&d);
    c["records"]["export_config"]["retained"] = json!("yes");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL malformed_companion: records.export_config.retained"
    );
}

#[test]
fn a_repeated_candidate_manifest_key_is_refused() {
    let d = scratch(SYN, "cand_dup");
    let p = path_of(&d, "candidate_manifest");
    let t = std::fs::read_to_string(&p).unwrap();
    // A second root `schema` before the real one: a first-wins and a last-wins reader agree on the
    // value here, so only the repeat itself can be refused.
    std::fs::write(&p, format!("schema = \"CandidateManifestV1\"\n{t}")).unwrap();
    restamp(&d, "candidate_manifest");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: candidate manifest repeats a key"
    );
}
