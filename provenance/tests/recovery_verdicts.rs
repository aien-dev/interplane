//! Recovery outcomes through the verifier (VAC M4d). An effect the daemon's ledger does not show
//! as landed must never verify as the DONE run does, and each of the three cases has its own
//! named refusal. The fixtures are the committed real and synthetic ones, mutated and restamped,
//! so the only difference from the passing bundle is the ack state (or the ack's absence).
//! The mutations are SYNTHETIC: the ack texts are the DONE ack with `state` changed, not an ack
//! the daemon wrote after a real interruption. Specification: provenance/BINDING.md.

mod common;

use common::*;
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::Path;

const REAL: &str = "fixtures/real-waldo-aien-chain";
const FIX: &str = "fixtures/synthetic-fix-the-test";
const SYN: &str = "fixtures/synthetic-full-chain";

fn set_state(d: &Path, state: &str) {
    let p = path_of(d, "ledger_ack");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let mut t: Value = serde_json::from_str(v["text"].as_str().unwrap()).unwrap();
    t["state"] = json!(state);
    v["text"] = json!(t.to_string());
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, "ledger_ack");
}

/// Intent with no ack: the ack is neither in the manifest nor on disk.
fn drop_ack(d: &Path) {
    let p = path_of(d, "ledger_ack");
    let mut c = companion(d);
    c["records"].as_object_mut().unwrap().remove("ledger_ack");
    write_companion(d, &c);
    std::fs::remove_file(p).unwrap();
}

fn starts(d: &Path, want: &str) -> String {
    let v = verify(d);
    assert!(v.starts_with(want), "wanted {want}, got {v}");
    v
}

#[test]
fn real_slice_done_is_pass_complete_and_the_three_unlanded_cases_are_not() {
    let done = verify(&scratch(REAL, "m4d-real-done"));
    assert!(done.starts_with("PASS complete"), "{done}");

    let d = scratch(REAL, "m4d-real-notdone");
    set_state(&d, "NOT_DONE");
    let nd = starts(&d, "FAIL effect_not_done:");

    let d = scratch(REAL, "m4d-real-unresolved");
    set_state(&d, "UNRESOLVED");
    let un = starts(&d, "FAIL effect_unresolved:");

    let d = scratch(REAL, "m4d-real-noack");
    drop_ack(&d);
    let ni = starts(&d, "FAIL effect_interrupted:");

    // Four different verdicts, none of them a PASS.
    let all = [done, nd, un, ni];
    for i in 0..all.len() {
        for j in 0..i {
            assert_ne!(all[i], all[j]);
        }
    }
}

#[test]
fn synthetic_slice_has_the_same_three_refusals() {
    let d = scratch(SYN, "m4d-syn-notdone");
    set_state(&d, "NOT_DONE");
    starts(&d, "FAIL effect_not_done:");
    let d = scratch(SYN, "m4d-syn-unresolved");
    set_state(&d, "UNRESOLVED");
    starts(&d, "FAIL effect_unresolved:");
    let d = scratch(SYN, "m4d-syn-noack");
    drop_ack(&d);
    starts(&d, "FAIL effect_interrupted:");
}

#[test]
fn a_test_run_on_top_of_an_unlanded_effect_is_refused() {
    let ok = verify(&scratch(FIX, "m4d-fix-done"));
    assert!(ok.starts_with("PASS_LABELLED_INCOMPLETE"), "{ok}");
    for (name, state) in [
        ("nd", Some("NOT_DONE")),
        ("un", Some("UNRESOLVED")),
        ("ni", None),
    ] {
        let d = scratch(FIX, &format!("m4d-fix-{name}"));
        match state {
            Some(s) => set_state(&d, s),
            None => drop_ack(&d),
        }
        let v = verify(&d);
        assert!(v.starts_with("FAIL test_run_on_unlanded_effect:"), "{v}");
        let inner = match state {
            Some("NOT_DONE") => "effect_not_done",
            Some(_) => "effect_unresolved",
            None => "effect_interrupted",
        };
        assert!(v.contains(inner), "{v}");
    }
}

#[test]
fn test_records_without_a_section_on_an_unlanded_effect_are_still_refused_as_such() {
    let d = scratch(FIX, "m4d-fix-nosection");
    set_state(&d, "NOT_DONE");
    let mut c = companion(&d);
    c.as_object_mut().unwrap().remove("test_run");
    write_companion(&d, &c);
    starts(&d, "FAIL test_run_on_unlanded_effect:");
}

#[test]
fn an_unknown_ack_state_is_still_a_generic_mismatch() {
    for s in ["OPEN", "done", "WRITTEN"] {
        let d = scratch(REAL, &format!("m4d-real-unknown-{s}"));
        set_state(&d, s);
        let v = verify(&d);
        assert_eq!(v, format!("FAIL binding_mismatch: ledger_ack.state={s}"));
    }
}

#[test]
fn an_interrupted_bundle_still_needs_its_earlier_records_to_agree() {
    // A forged claim next to a missing ack is reported as the forgery, not as an interruption.
    let d = scratch(REAL, "m4d-real-noack-badintent");
    drop_ack(&d);
    let p = path_of(&d, "ledger_intent");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["verified"] = json!(false);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(&d, "ledger_intent");
    starts(&d, "FAIL binding_mismatch: ledger_intent.verified");
}
