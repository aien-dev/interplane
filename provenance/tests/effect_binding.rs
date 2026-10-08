//! Effect binding tests: the ledger slice (`aien-ledger-slice/1`, strong) and the loose
//! `record_effect_receipt/1` (weaker). Every expected verdict is a literal. Each mutated archive is
//! internally restamped (record digests recomputed) so only the binding itself can refuse it.
//! Specification: provenance/BINDING.md.

mod common;

use common::*;
use interplane_provenance::binding::{
    approval_binding_bytes, compact_sorted, ApprovalFields, APPROVAL_BINDING,
};
use interplane_provenance::gojson::sha256_hex;
use interplane_provenance::synth::{self, Opts};
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SYN: &str = "fixtures/synthetic-full-chain";
const WEAK: &str = "fixtures/synthetic-weak-receipt";
const STRONG: &str = "effect=aien-ledger-slice/1:strong proposal=scripted_turn";
const WEAKL: &str = "effect=record_effect_receipt/1:weak";
const INCOMPLETE: &str = "PASS_LABELLED_INCOMPLETE missing=link:model_turn";

fn fresh(name: &str, o: &Opts) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("effect")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    synth::write_synthetic(&d, o).unwrap();
    d
}

/// Change the JSON inside a record's `text` string, keep the rest of the exported view, restamp.
fn edit_text(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let mut t: Value = serde_json::from_str(v["text"].as_str().unwrap()).unwrap();
    f(&mut t);
    v["text"] = json!(t.to_string());
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

/// Change the exported view itself (id, verified, note, ...), restamp.
fn edit_view(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

fn edit_json(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    edit_view(d, rec, f)
}

/// Copy `from`'s record `rec` over `to`'s and restamp.
fn transplant(from: &Path, to: &Path, rec: &str) {
    std::fs::copy(path_of(from, rec), path_of(to, rec)).unwrap();
    restamp(to, rec);
}

const LEDGER: [&str; 5] = [
    "ledger_claim",
    "ledger_committed",
    "ledger_grant",
    "ledger_intent",
    "ledger_ack",
];

fn with_slice_of(name: &str, base: &Opts, donor: &Opts) -> PathBuf {
    let d = fresh(name, base);
    let other = fresh(&format!("{name}-donor"), donor);
    for r in LEDGER {
        transplant(&other, &d, r);
    }
    d
}

fn asset(opts: impl FnOnce(&mut Opts)) -> Opts {
    let mut o = Opts::default();
    opts(&mut o);
    o
}

// ---------- labels ----------

#[test]
fn ledger_slice_is_labelled_strong() {
    assert_eq!(verify(Path::new(SYN)), format!("{INCOMPLETE} {STRONG}"));
}

#[test]
fn record_effect_receipt_v1_is_still_accepted_and_labelled_weaker() {
    assert_eq!(verify(Path::new(WEAK)), format!("{INCOMPLETE} {WEAKL}"));
}

// ---------- identity: request, trace, model ----------

#[test]
fn ledger_for_another_request_than_the_link_names() {
    // Request 02 is in the trace, so the trace check passes; the daemon's records name request 01.
    let d = scratch(SYN, "ledger_other_request");
    let mut c = companion(&d);
    c["interplane"]["request_id"] = json!("call-prov-02");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_grant.request_id ledger=call-prov-01 link=call-prov-02"
    );
}

#[test]
fn ledger_from_another_trace() {
    // Same request id, same arguments, same everything except the trace id.
    let d = with_slice_of(
        "ledger_other_trace",
        &Opts::default(),
        &asset(|o| o.trace_id = Some("trace-prov-02".into())),
    );
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_grant.trace_id ledger=trace-prov-02 link=trace-prov-01"
    );
}

#[test]
fn claim_for_another_trace_than_its_grant() {
    let d = scratch(SYN, "claim_other_trace");
    edit_text(&d, "ledger_claim", |t| {
        t["trace_id"] = json!("trace-prov-02")
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_claim.trace_id ledger=trace-prov-02 link=trace-prov-01"
    );
}

#[test]
fn effect_from_a_different_daemon_process_than_the_one_that_loaded_the_model() {
    let d = scratch(SYN, "other_daemon_process");
    edit_json(&d, "daemon_run", |r| r["pid"] = json!(4243));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_claim.executor.pid vs daemon_run.pid"
    );
    let d = scratch(SYN, "other_daemon_start");
    edit_json(&d, "daemon_run", |r| r["start_ticks"] = json!(1));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_claim.executor.start vs daemon_run.start_ticks"
    );
}

#[test]
fn effect_from_a_different_executable_than_the_aien_link() {
    let d = scratch(SYN, "other_executable");
    edit_json(&d, "daemon_run", |r| {
        r["executable_sha256"] = json!("0".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: daemon_run.executable_sha256 vs aien.executable_sha256"
    );
}

#[test]
fn effect_from_a_daemon_that_bound_another_socket_than_the_load_log() {
    let d = scratch(SYN, "other_socket");
    edit_json(&d, "daemon_run", |r| r["socket"] = json!("/elsewhere.sock"));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: aien_load.socket vs daemon_run.socket"
    );
}

#[test]
fn load_log_for_another_model_than_the_export() {
    let d = scratch(SYN, "other_model");
    let p = path_of(&d, "aien_load_log");
    let c = companion(&d);
    let sha = c["records"]["export_weights"]["sha256"].as_str().unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, text.replace(sha, &"1".repeat(64))).unwrap();
    restamp(&d, "aien_load_log");
    assert_eq!(verify(&d), "FAIL binding_mismatch: aien_load.model_sha256");
}

// ---------- altered record, one test per record type ----------

#[test]
fn altered_claim() {
    let d = scratch(SYN, "alter_claim");
    edit_text(&d, "ledger_claim", |t| {
        t["approval_key"] = json!("a".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_claim.approval_key"
    );
}

#[test]
fn altered_committed() {
    let d = scratch(SYN, "alter_committed");
    edit_text(&d, "ledger_committed", |t| {
        t["evidence"]["cx_promotion"] = json!(999)
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_committed.evidence.cx_promotion"
    );
}

#[test]
fn committed_for_another_claim() {
    let d = scratch(SYN, "committed_other_claim");
    edit_text(&d, "ledger_committed", |t| t["claim"] = json!(7));
    assert_eq!(verify(&d), "FAIL binding_mismatch: ledger_committed.claim");
}

#[test]
fn altered_grant() {
    // The grant's approved content changed: the approval key recomputed from its own fields no
    // longer equals the key the claim recorded.
    let d = scratch(SYN, "alter_grant");
    edit_text(&d, "ledger_grant", |t| {
        t["content_sha256"] = json!("b".repeat(64))
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger_grant.approval_key (recomputed from its approval fields)"
    );
}

#[test]
fn altered_intent() {
    let d = scratch(SYN, "alter_intent");
    edit_text(&d, "ledger_intent", |t| t["path"] = json!("notes/evil.md"));
    assert_eq!(verify(&d), "FAIL binding_mismatch: ledger_intent.path");
}

#[test]
fn altered_ack() {
    let d = scratch(SYN, "alter_ack");
    edit_text(&d, "ledger_ack", |t| t["state"] = json!("NOT_DONE"));
    assert_eq!(verify(&d), "FAIL effect_not_done: ledger_ack.state=NOT_DONE: the daemon read the world back and the write did not land");
    let d = scratch(SYN, "alter_ack_disk");
    edit_text(&d, "ledger_ack", |t| {
        t["disk_sha256"] = json!("c".repeat(64))
    });
    assert_eq!(verify(&d), "FAIL binding_mismatch: ledger_ack.disk_sha256");
}

#[test]
fn record_the_daemon_did_not_verify() {
    let d = scratch(SYN, "unverified_grant");
    edit_view(&d, "ledger_grant", |v| v["verified"] = json!(false));
    assert_eq!(verify(&d), "FAIL binding_mismatch: ledger_grant.verified");
}

#[test]
fn altered_record_without_restamp_is_a_digest_mismatch() {
    let d = scratch(SYN, "alter_no_restamp");
    let p = path_of(&d, "ledger_ack");
    let mut b = std::fs::read(&p).unwrap();
    let n = b.len() / 2;
    b[n] ^= 0x01;
    std::fs::write(&p, b).unwrap();
    assert_eq!(verify(&d), "FAIL digest_mismatch: ledger_ack");
}

#[test]
fn records_out_of_ledger_order() {
    let d = scratch(SYN, "order");
    // The ack claims to predate its intent.
    edit_view(&d, "ledger_ack", |v| v["id"] = json!(25));
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: ledger record order [20, 26, 27, 28, 25]"
    );
}

#[test]
fn ledger_record_removed_from_the_manifest() {
    let d = scratch(SYN, "no_ack");
    let mut c = companion(&d);
    c["records"].as_object_mut().unwrap().remove("ledger_ack");
    write_companion(&d, &c);
    assert_eq!(verify(&d), "FAIL effect_interrupted: ledger_intent is the last record: no ack, the write may or may not have landed");
}

// ---------- INTERPLANE call against the ledger ----------

#[test]
fn interplane_arguments_differ_from_what_the_daemon_approved() {
    let d = scratch(SYN, "args_differ");
    edit_json(&d, "interplane_trace", |t| {
        t[0]["payload"]["arguments"]["content"] = json!("hello world!")
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: interplane.arguments.content request=call-prov-01"
    );
}

#[test]
fn interplane_result_names_other_ledger_records() {
    let d = scratch(SYN, "result_ids");
    edit_json(&d, "interplane_trace", |t| {
        t[1]["payload"]["data"]["receipt"]["grant_id"] = json!(1)
    });
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: interplane.result.data.receipt.grant_id request=call-prov-01"
    );
}

// ---------- identical calls ----------

#[test]
fn two_identical_calls_in_one_trace_are_told_apart() {
    let a = fresh("same_trace_call_01", &Opts::default());
    let b = fresh(
        "same_trace_call_03",
        &asset(|o| o.request_id = Some(synth::IDENTICAL_REQUEST_ID.into())),
    );
    // The two calls have the same tool, arguments and result data.
    let ta: Value =
        serde_json::from_slice(&std::fs::read(path_of(&a, "interplane_trace")).unwrap()).unwrap();
    assert_eq!(ta[0]["payload"]["arguments"], ta[4]["payload"]["arguments"]);
    assert_eq!(ta[0]["payload"]["tool"], ta[4]["payload"]["tool"],);
    // Each verifies against its own slice ...
    assert_eq!(verify(&a), format!("{INCOMPLETE} {STRONG}"));
    assert_eq!(verify(&b), format!("{INCOMPLETE} {STRONG}"));
    // ... and not against the other's.
    let swapped = with_slice_of(
        "same_trace_swapped",
        &Opts::default(),
        &asset(|o| o.request_id = Some(synth::IDENTICAL_REQUEST_ID.into())),
    );
    assert_eq!(
        verify(&swapped),
        "FAIL binding_mismatch: ledger_grant.request_id ledger=call-prov-03 link=call-prov-01"
    );
}

#[test]
fn two_identical_calls_in_two_traces_are_told_apart() {
    let a = fresh("two_traces_a", &Opts::default());
    let b = fresh(
        "two_traces_b",
        &asset(|o| o.trace_id = Some("trace-prov-02".into())),
    );
    assert_eq!(verify(&a), format!("{INCOMPLETE} {STRONG}"));
    assert_eq!(verify(&b), format!("{INCOMPLETE} {STRONG}"));
    let swapped = with_slice_of(
        "two_traces_swapped",
        &asset(|o| o.trace_id = Some("trace-prov-02".into())),
        &Opts::default(),
    );
    assert_eq!(
        verify(&swapped),
        "FAIL binding_mismatch: ledger_grant.trace_id ledger=trace-prov-01 link=trace-prov-02"
    );
}

#[test]
fn weaker_receipt_cannot_tell_two_identical_calls_apart_and_says_so() {
    let a = fresh("weak_call_01", &asset(|o| o.weak = true));
    let b = fresh(
        "weak_call_03",
        &asset(|o| {
            o.weak = true;
            o.request_id = Some(synth::IDENTICAL_REQUEST_ID.into())
        }),
    );
    // Byte-identical receipts: nothing in them names the request or the trace.
    assert_eq!(
        std::fs::read(path_of(&a, "effect_receipt")).unwrap(),
        std::fs::read(path_of(&b, "effect_receipt")).unwrap()
    );
    transplant(&b, &a, "effect_receipt");
    // Call 03's receipt is accepted for call 01. The verdict carries the weaker label.
    assert_eq!(verify(&a), format!("{INCOMPLETE} {WEAKL}"));
}

// ---------- canonicalization ----------

#[test]
fn receipt_declared_in_the_jcs_form_is_refused() {
    let d = scratch(WEAK, "digest_form_jcs");
    let mut c = companion(&d);
    c["effect"]["digest_form"] = json!("jcs-rfc8785");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL canonicalization_mismatch: effect.digest_form=jcs-rfc8785 (producer form is serde_json.to_vec.sorted-keys/1)"
    );
    let d = scratch(WEAK, "digest_form_absent");
    let mut c = companion(&d);
    c["effect"].as_object_mut().unwrap().remove("digest_form");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL canonicalization_mismatch: effect.digest_form=absent (producer form is serde_json.to_vec.sorted-keys/1)"
    );
}

fn jcs(v: &Value) -> String {
    format!(
        "sha256:{}",
        sha256_hex(interplane_core::canonicalize(v).as_bytes())
    )
}

/// Rewrite the weak fixture's request arguments (trace) and put `digest(args)` in its receipt.
fn weak_with_args(name: &str, args: Value, digest: impl Fn(&Value) -> String) -> PathBuf {
    let d = scratch(WEAK, name);
    edit_json(&d, "interplane_trace", |t| {
        t[0]["payload"]["arguments"] = args.clone()
    });
    edit_json(&d, "effect_receipt", |r| {
        r["arguments_digest"] = json!(digest(&args))
    });
    d
}

#[test]
fn receipt_hashed_in_the_jcs_form_is_refused_by_name() {
    // JCS orders keys by UTF-16 code unit, serde_json by UTF-8 byte: the astral key sorts first
    // in one and last in the other. A shared "sorted keys" reading would hide this.
    let args = json!({"\u{ffff}": 1, "\u{1f600}": 2});
    assert_ne!(
        jcs(&args),
        format!("sha256:{}", sha256_hex(&compact_sorted(&args).unwrap()))
    );
    let d = weak_with_args("jcs_hashed", args, jcs);
    assert_eq!(
        verify(&d),
        "FAIL canonicalization_mismatch: effect_receipt.arguments_digest is the JCS form; the producer form is serde_json.to_vec.sorted-keys/1"
    );
}

#[test]
fn receipt_in_the_producer_form_is_accepted_where_jcs_would_differ() {
    let args = json!({"\u{ffff}": 1, "\u{1f600}": 2});
    let d = weak_with_args("producer_hashed", args, |v| {
        format!("sha256:{}", sha256_hex(&compact_sorted(v).unwrap()))
    });
    assert_eq!(verify(&d), format!("{INCOMPLETE} {WEAKL}"));
}

#[test]
fn float_arguments_are_refused_not_approximated() {
    let args = json!({"x": 1.5});
    let d = weak_with_args("float_args", args, |_| "sha256:00".into());
    assert_eq!(
        verify(&d),
        "FAIL canonicalization_mismatch: effect_receipt.arguments_digest: producer form serde_json.to_vec.sorted-keys/1 not reproduced: non-integer number 1.5"
    );
}

#[test]
fn unknown_effect_binding_is_refused() {
    let d = scratch(SYN, "binding_v2");
    let mut c = companion(&d);
    c["effect"]["binding"] = json!("aien-ledger-slice/2");
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unsupported_binding: effect.binding=aien-ledger-slice/2"
    );
    let d = scratch(SYN, "binding_absent");
    let mut c = companion(&d);
    c["effect"].as_object_mut().unwrap().remove("binding");
    c["completeness"] = json!({"state": "complete", "missing": []});
    write_companion(&d, &c);
    assert_eq!(verify(&d), "FAIL malformed_companion: effect.binding");
    let d = scratch(SYN, "approval_binding_v1");
    let mut c = companion(&d);
    c["effect"]["approval_binding"] = json!("aien.approval.v1");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unsupported_binding: effect.approval_binding != aien.approval.v2 (got aien.approval.v1)"
    );
}

// ---------- the byte forms themselves ----------

#[test]
fn compact_sorted_equals_serde_json_to_vec_for_integer_values() {
    let v = json!({
        "z": [1, -2, 3000000000u64, null, true, false],
        "a": {"b\"c": "q\\r\n\t\u{8}\u{c}\u{1}\u{1f} \u{7f} \u{2028} é \u{1f600}", "A": 18446744073709551615u64},
        "": "",
        "\u{ffff}": 1, "\u{1f600}": 2
    });
    assert_eq!(compact_sorted(&v).unwrap(), serde_json::to_vec(&v).unwrap());
}

#[test]
fn approval_binding_bytes_are_the_daemons_sorted_compact_object() {
    // sovereign-core `binding_bytes`: a serde_json map of nine strings plus "v", compact, sorted.
    let f = ApprovalFields {
        approval_id: "aien-approval:w1:c32f",
        approved_proposal_sha256: "f766",
        approver: "dr\"ake",
        content_sha256: "7a65",
        desk_key_id: "440f",
        path: "dir/summary.txt",
        request_id: "w1",
        trace_id: "ledger",
        workspace: "/w s/\u{e9}",
    };
    let expect = json!({
        "approval_id": f.approval_id, "approved_proposal_sha256": f.approved_proposal_sha256,
        "approver": f.approver, "content_sha256": f.content_sha256, "desk_key_id": f.desk_key_id,
        "path": f.path, "request_id": f.request_id, "trace_id": f.trace_id,
        "workspace": f.workspace, "v": APPROVAL_BINDING,
    });
    assert_eq!(
        approval_binding_bytes(&f),
        serde_json::to_vec(&expect).unwrap()
    );
}

// ---------- evidence, never authority ----------

#[test]
fn verifying_reads_and_changes_nothing() {
    let d = scratch(SYN, "read_only");
    let snapshot = |d: &Path| {
        let mut all = Vec::new();
        let mut stack = vec![d.to_path_buf()];
        while let Some(p) = stack.pop() {
            for e in std::fs::read_dir(&p).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    all.push((e.path(), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        all.sort();
        all
    };
    let before = snapshot(&d);
    let verdict = verify(&d);
    assert!(verdict.starts_with("PASS_LABELLED_INCOMPLETE"), "{verdict}");
    assert_eq!(snapshot(&d), before);
    // A PASS is a statement about records. No verdict text carries a permission.
    for word in ["allow", "grant ", "authoriz", "permit", "execute"] {
        assert!(!verdict.to_lowercase().contains(word), "{verdict}");
    }
}
