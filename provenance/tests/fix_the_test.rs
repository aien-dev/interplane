//! The fix-the-test slice: `vac-test-run/1` (test run), the source pin and the `aien.native`
//! claim. Every expected verdict is a literal. Mutated archives are restamped (record digests
//! recomputed), so only the new checks can refuse them. Specification: provenance/BINDING.md.

mod common;

use common::*;
use interplane_provenance::synth::{self, Opts};
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const FIX: &str = "fixtures/synthetic-fix-the-test";
const STRONG: &str = "effect=aien-ledger-slice/1:strong proposal=scripted_turn";
const BLOB_OTHER: &str = "9999999999999999999999999999999999999999999999999999999999999999";

fn variant(name: &str, native: Option<bool>) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("fixthetest")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    synth::write_synthetic(
        &d,
        &Opts {
            fix_the_test: true,
            native,
            ..Default::default()
        },
    )
    .unwrap();
    d
}

fn edit(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

fn edit_companion(d: &Path, f: impl FnOnce(&mut Value)) {
    let mut c = companion(d);
    f(&mut c);
    write_companion(d, &c);
}

fn declare_missing(c: &mut Value, items: &[&str]) {
    c["completeness"]["missing"] = json!(items);
}

fn code(d: &Path) -> String {
    verify(d)
}

fn refused(d: &Path, want: &str) {
    let v = code(d);
    assert!(
        v.starts_with(&format!("FAIL {want}:")),
        "wanted {want}, got {v}"
    );
}

#[test]
fn committed_fixture_is_labelled_incomplete_with_native_proven() {
    let d = scratch(FIX, "ftt-committed");
    assert_eq!(
        code(&d),
        format!("PASS_LABELLED_INCOMPLETE missing=link:model_turn {STRONG}")
    );
}

#[test]
fn committed_fixture_matches_the_generator() {
    let d = variant("regen", Some(true));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIX);
    assert_eq!(
        std::fs::read(d.join("COMPANION.json")).unwrap(),
        std::fs::read(root.join("COMPANION.json")).unwrap()
    );
}

#[test]
fn absent_native_field_is_labelled_missing_never_complete() {
    let d = variant("no-native", None);
    assert_eq!(
        code(&d),
        format!("PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native {STRONG}")
    );
}

#[test]
fn native_claimed_false_is_labelled_missing() {
    let d = variant("stub", Some(false));
    assert_eq!(
        code(&d),
        format!("PASS_LABELLED_INCOMPLETE missing=link:model_turn,link:native {STRONG}")
    );
}

#[test]
fn declaring_native_missing_is_required_to_match() {
    // Dropping `link:native` from the declared list while no native proof exists is unlabelled.
    let d = variant("undeclared", None);
    edit_companion(&d, |c| {
        c["completeness"]["missing"] = json!(["link:model_turn"])
    });
    refused(&d, "completeness_mismatch");
}

// ---- tamper tests -------------------------------------------------------------------------

#[test]
fn changed_exit_code_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-exit");
    edit(&d, "test_run_record", |v| v["exit_code"] = json!(1));
    refused(&d, "test_run_mismatch");
}

#[test]
fn test_that_already_passed_before_the_fix_is_refused() {
    let d = scratch(FIX, "ftt-before");
    edit(&d, "test_run_record", |v| v["test_exit_before"] = json!(0));
    refused(&d, "test_run_mismatch");
}

#[test]
fn swapped_stdout_digest_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-stdout-digest");
    edit(&d, "test_run_record", |v| {
        v["stdout_sha256"] = json!(BLOB_OTHER)
    });
    refused(&d, "test_run_mismatch");
}

#[test]
fn swapped_stdout_file_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-stdout-file");
    let p = path_of(&d, "test_stdout");
    std::fs::remove_file(&p).unwrap();
    std::fs::write(&p, b"ok   everything passed, trust me\n").unwrap();
    restamp(&d, "test_stdout");
    refused(&d, "test_run_mismatch");
}

#[test]
fn wrong_target_blob_after_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-blob");
    edit(&d, "test_run_record", |v| {
        v["target_blob_sha256_after"] = json!(BLOB_OTHER)
    });
    refused(&d, "test_run_mismatch");
}

#[test]
fn argv_other_than_the_task_command_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-argv");
    edit(&d, "test_run_record", |v| v["argv"] = json!(["true"]));
    refused(&d, "test_run_mismatch");
}

#[test]
fn task_id_disagreement_is_a_test_run_mismatch() {
    let d = scratch(FIX, "ftt-taskid");
    edit_companion(&d, |c| c["test_run"]["task_id"] = json!("another-task"));
    refused(&d, "test_run_mismatch");
}

#[test]
fn dropped_test_run_section_is_test_run_missing() {
    let d = scratch(FIX, "ftt-dropped");
    edit_companion(&d, |c| {
        c.as_object_mut().unwrap().remove("test_run");
    });
    refused(&d, "test_run_missing");
}

#[test]
fn dropped_test_run_record_is_test_run_missing() {
    let d = scratch(FIX, "ftt-dropped-record");
    edit_companion(&d, |c| {
        c["records"]
            .as_object_mut()
            .unwrap()
            .remove("test_run_record");
    });
    refused(&d, "test_run_missing");
}

#[test]
fn tampered_source_pin_blob_is_a_source_pin_mismatch() {
    let d = scratch(FIX, "ftt-pin-blob");
    edit(&d, "source_pin", |v| {
        v["target_blob_sha256"] = json!(BLOB_OTHER)
    });
    refused(&d, "source_pin_mismatch");
}

#[test]
fn tampered_source_pin_path_and_commit_are_source_pin_mismatches() {
    let d = scratch(FIX, "ftt-pin-path");
    edit(&d, "source_pin", |v| {
        v["target_path"] = json!("src/other.c")
    });
    refused(&d, "source_pin_mismatch");
    let d = scratch(FIX, "ftt-pin-commit");
    edit(&d, "source_pin", |v| v["commit"] = json!("abc"));
    refused(&d, "source_pin_mismatch");
}

#[test]
fn unsupported_test_run_binding_or_version_is_refused() {
    let d = scratch(FIX, "ftt-binding");
    edit_companion(&d, |c| c["test_run"]["binding"] = json!("vac-test-run/2"));
    refused(&d, "unsupported_test_run");
    let d = scratch(FIX, "ftt-version");
    edit(&d, "test_run_record", |v| v["v"] = json!(2));
    refused(&d, "unsupported_test_run");
}

// ---- native claim -------------------------------------------------------------------------

#[test]
fn forged_native_true_against_a_stub_recall_is_contradicted() {
    let d = variant("forged-stub", Some(false));
    edit_companion(&d, |c| {
        c["aien"]["native"] = json!({"claimed": true, "omega_sha": synth::OMEGA_SHA});
        declare_missing(c, &["link:model_turn"]);
    });
    refused(&d, "native_claim_contradicted");
}

#[test]
fn forged_native_true_with_no_recall_report_is_contradicted() {
    let d = variant("forged-absent", None);
    edit_companion(&d, |c| {
        c["aien"]["native"] = json!({"claimed": true, "omega_sha": synth::OMEGA_SHA});
        declare_missing(c, &["link:model_turn"]);
    });
    refused(&d, "native_claim_contradicted");
}

#[test]
fn forged_native_sha_differing_from_the_recall_is_contradicted() {
    let d = scratch(FIX, "ftt-native-sha");
    edit_companion(&d, |c| {
        c["aien"]["native"]["omega_sha"] = json!("0000000000000000000000000000000000000000");
    });
    refused(&d, "native_claim_contradicted");
}

#[test]
fn native_false_while_the_recall_says_native_is_contradicted() {
    let d = scratch(FIX, "ftt-native-under");
    edit_companion(&d, |c| {
        c["aien"]["native"]["claimed"] = json!(false);
        declare_missing(c, &["link:model_turn", "link:native"]);
    });
    refused(&d, "native_claim_contradicted");
}

#[test]
fn malformed_native_is_malformed() {
    let d = scratch(FIX, "ftt-native-malformed");
    edit_companion(&d, |c| {
        c["aien"]["native"] = json!({"claimed": "yes"});
        declare_missing(c, &["link:model_turn", "link:native"]);
    });
    refused(&d, "malformed_companion");
}

#[test]
fn test_run_needs_the_strong_ledger_binding() {
    let d = variant("weak-ledger", Some(true));
    edit_companion(&d, |c| {
        c["effect"]["binding"] = json!("record_effect_receipt/1")
    });
    let v = code(&d);
    assert!(v.starts_with("FAIL "), "{v}");
}

#[test]
fn verifying_the_slice_changes_nothing() {
    let d = scratch(FIX, "ftt-readonly");
    let before = std::fs::read(d.join("COMPANION.json")).unwrap();
    let _ = verify(&d);
    assert_eq!(before, std::fs::read(d.join("COMPANION.json")).unwrap());
}

#[test]
fn write_outside_the_task_target_is_a_task_scope_mismatch() {
    // The task allows src/clamp.c; here it names the test file instead, so the grant's write
    // (src/clamp.c) is not the file the task allows. The same check refuses a run whose grant
    // writes the test file while the task names the code.
    let d = scratch(FIX, "ftt-scope");
    edit(&d, "task", |v| {
        v["target_path"] = json!("test/test_clamp.c")
    });
    refused(&d, "task_scope_mismatch");
}

#[test]
fn task_without_a_target_path_is_a_task_scope_mismatch() {
    let d = scratch(FIX, "ftt-scope-absent");
    edit(&d, "task", |v| {
        v.as_object_mut().unwrap().remove("target_path");
    });
    refused(&d, "task_scope_mismatch");
}

#[test]
fn stderr_digest_claimed_but_stderr_file_missing_is_refused() {
    let d = scratch(FIX, "ftt-stderr-missing");
    let p = path_of(&d, "test_stderr");
    std::fs::remove_file(&p).unwrap();
    // Drop the record entry too: the digest alone must not pass.
    let mut c = companion(&d);
    let recs = c["records"].as_object_mut().expect("records object");
    recs.remove("test_stderr");
    write_companion(&d, &c);
    refused(&d, "test_run_mismatch");
}

#[test]
fn stderr_digest_claimed_but_stderr_not_retained_is_refused() {
    let d = scratch(FIX, "ftt-stderr-unretained");
    let mut c = companion(&d);
    let r = &mut c["records"]["test_stderr"];
    r.as_object_mut().unwrap().remove("path");
    r["retained"] = json!(false);
    c["completeness"]["missing"]
        .as_array_mut()
        .unwrap()
        .push(json!("record:test_stderr"));
    write_companion(&d, &c);
    refused(&d, "test_run_mismatch");
}

#[test]
fn stderr_altered_by_one_byte_is_refused() {
    let d = scratch(FIX, "ftt-stderr-flip");
    let p = path_of(&d, "test_stderr");
    let mut b = std::fs::read(&p).unwrap();
    if b.is_empty() {
        b.push(b'x');
    } else {
        b[0] ^= 1;
    }
    std::fs::write(&p, b).unwrap();
    restamp(&d, "test_stderr");
    refused(&d, "test_run_mismatch");
}

#[test]
fn intact_stderr_still_passes() {
    let d = scratch(FIX, "ftt-stderr-ok");
    assert!(code(&d).starts_with("PASS_LABELLED_INCOMPLETE"));
}

#[test]
fn stdout_digest_claimed_but_stdout_not_retained_is_refused() {
    let d = scratch(FIX, "ftt-stdout-unretained");
    let mut c = companion(&d);
    let r = &mut c["records"]["test_stdout"];
    r.as_object_mut().unwrap().remove("path");
    r["retained"] = json!(false);
    c["completeness"]["missing"]
        .as_array_mut()
        .unwrap()
        .push(json!("record:test_stdout"));
    write_companion(&d, &c);
    refused(&d, "test_run_mismatch");
}
