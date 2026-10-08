//! `rsi-eval/2`: the independent judge's signed evaluation of the exact change the daemon wrote.
//! Every expected verdict is a literal. Records are added to (or mutated in) copies of the
//! synthetic fix-the-test bundle and restamped, so only the evaluation checks can refuse them.
//! Specification: provenance/BINDING.md ("Independent evaluation").

mod common;

use common::*;
use interplane_provenance::evaluation::{parse_judge_key, precheck, receipt_digest_v2, Expect};
use interplane_provenance::gojson::sha256_hex;
use interplane_provenance::synth::{self, Opts};
use interplane_provenance::{verify, verify_with, Options};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const STRONG: &str = "effect=aien-ledger-slice/1:strong proposal=scripted_turn";
const OTHER: &str = "9999999999999999999999999999999999999999999999999999999999999999";
const HOLDOUTS: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).unwrap()
}

fn judge() -> SigningKey {
    key(7)
}

fn opts(k: &SigningKey) -> Options {
    Options {
        judge_key: Some(*k.verifying_key()),
        ..Default::default()
    }
}

fn base() -> PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("evaluation")
            .join("base");
        let _ = std::fs::remove_dir_all(&d);
        synth::write_synthetic(
            &d,
            &Opts {
                fix_the_test: true,
                native: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        d
    })
    .clone()
}

fn fresh(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("evaluation")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    copy_dir(&base(), &d);
    d
}

fn record_json(d: &Path, rec: &str) -> Value {
    serde_json::from_slice(&std::fs::read(path_of(d, rec)).unwrap()).unwrap()
}

/// The written path and its digest, as the bundle's own task and test run state them.
fn subject(d: &Path) -> (String, String) {
    let task = record_json(d, "task");
    let run = record_json(d, "test_run_record");
    (
        task["target_path"].as_str().unwrap().to_string(),
        run["target_blob_sha256_after"]
            .as_str()
            .unwrap()
            .to_string(),
    )
}

fn policy(path: &str) -> Value {
    json!({"policy_id": "test", "holdout_set_sha256": HOLDOUTS, "min_holdout_pass_ratio": 1.0,
        "require_admitted": true, "allowed_targets": [path], "protected_paths": ["holdouts"]})
}

fn sign(r: &mut Value, k: &SigningKey) {
    let digest = receipt_digest_v2(r).unwrap();
    let sig: Signature = k.sign(&hex::decode(&digest).unwrap());
    r["receipt_digest"] = json!(digest);
    r["signature"] = json!(format!("tpm2-p256:{}", hex::encode(sig.to_bytes())));
}

fn receipt(path: &str, sha: &str, policy_bytes: &[u8], parent: &str) -> Value {
    json!({
        "cycle_id": "cyc", "candidate_id": "cand", "parent_id": parent,
        "evaluated_at": "2026-10-08T22:00:00Z", "evaluator_version": "1.0.0",
        "passed_all_hard_invariants": true, "passed_statistical_gates": true, "admitted": true,
        "layer_results": [
            {"layer_name": "correctness", "is_hard_invariant": true, "passed": true, "score": 1.0,
             "summary": "ok", "violations": []},
            {"layer_name": "performance", "is_hard_invariant": false, "passed": true,
             "score": 0.8333333333333334, "summary": "within margin", "violations": []}
        ],
        "metrics_summary": {"latency_delta_pct": -0.12345678901234568, "p_value": 0.5,
            "p95_ci_upper_degradation_pct": 0.1, "p99_ci_upper_degradation_pct": 0.2,
            "rss_growth_pct": 0.0, "candidate_resident_mb": 3},
        "receipt_digest": "", "signature": null, "format_version": 2,
        "binding": {"subject_path": path, "subject_sha256": sha,
            "holdout_set_sha256": HOLDOUTS, "holdouts_total": 4, "holdouts_passed": 4,
            "policy_sha256": sha256_hex(policy_bytes),
            "evaluator_binary_sha256": OTHER}
    })
}

/// Adds the evaluation records and section. `tweak` edits the receipt before it is signed.
fn add_evaluation(d: &Path, k: &SigningKey, tweak: impl FnOnce(&mut Value), pol: Option<Value>) {
    let (path, sha) = subject(d);
    let policy_bytes = serde_json::to_vec_pretty(&pol.unwrap_or_else(|| policy(&path))).unwrap();
    let parent = record_json(d, "source_pin")["commit"]
        .as_str()
        .unwrap()
        .to_string();
    let mut r = receipt(&path, &sha, &policy_bytes, &parent);
    tweak(&mut r);
    sign(&mut r, k);
    let dir = d.join("records/evaluation");
    std::fs::create_dir_all(&dir).unwrap();
    let rb = serde_json::to_vec_pretty(&r).unwrap();
    std::fs::write(dir.join("receipt.json"), &rb).unwrap();
    std::fs::write(dir.join("policy.json"), &policy_bytes).unwrap();
    let mut c = companion(d);
    for (n, p, b) in [
        ("evaluation_receipt", "records/evaluation/receipt.json", &rb),
        (
            "evaluation_policy",
            "records/evaluation/policy.json",
            &policy_bytes,
        ),
    ] {
        c["records"][n] = json!({"path": p, "sha256": sha256_hex(b), "bytes": b.len()});
    }
    c["evaluation"] = json!({"binding": "rsi-eval/2", "receipt": "evaluation_receipt",
        "policy": "evaluation_policy"});
    write_companion(d, &c);
}

fn edit(d: &Path, rec: &str, f: impl FnOnce(&mut Value)) {
    let p = path_of(d, rec);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    restamp(d, rec);
}

fn code(v: &str) -> &str {
    v.strip_prefix("FAIL ")
        .and_then(|r| r.split(':').next())
        .unwrap_or(v)
}

fn baseline() -> String {
    verify(&base())
}

#[test]
fn baseline_bundle_verifies_without_evaluation() {
    let v = baseline();
    assert!(v.starts_with("PASS_LABELLED_INCOMPLETE"), "{v}");
    assert!(v.ends_with(STRONG), "{v}");
}

#[test]
fn judged_bundle_passes_with_the_pinned_key() {
    let d = fresh("pass");
    add_evaluation(&d, &judge(), |_| {}, None);
    assert_eq!(verify_with(&d, &opts(&judge())), baseline());
}

#[test]
fn without_a_key_an_evaluation_is_untrusted() {
    let d = fresh("untrusted");
    add_evaluation(&d, &judge(), |_| {}, None);
    assert_eq!(code(&verify(&d)), "evaluation_untrusted");
}

#[test]
fn a_supplied_key_requires_an_evaluation() {
    assert_eq!(
        code(&verify_with(&base(), &opts(&judge()))),
        "evaluation_missing"
    );
    let d = fresh("dropped");
    add_evaluation(&d, &judge(), |_| {}, None);
    let mut c = companion(&d);
    c.as_object_mut().unwrap().remove("evaluation");
    write_companion(&d, &c);
    assert_eq!(
        code(&verify(&d)),
        "evaluation_missing",
        "records without a section"
    );
}

#[test]
fn version_1_receipts_are_insufficient() {
    let d = fresh("v1");
    add_evaluation(&d, &judge(), |_| {}, None);
    edit(&d, "evaluation_receipt", |r| {
        r["format_version"] = json!(1);
    });
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_v1_insufficient"
    );
    edit(&d, "evaluation_receipt", |r| {
        r.as_object_mut().unwrap().remove("format_version");
    });
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_v1_insufficient"
    );
}

#[test]
fn substituted_scores_break_the_signature() {
    let edits: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        (
            "count",
            Box::new(|r| r["binding"]["holdouts_passed"] = json!(3)),
        ),
        (
            "score",
            Box::new(|r| r["layer_results"][1]["score"] = json!(0.9)),
        ),
        (
            "admitted",
            Box::new(|r| r["passed_statistical_gates"] = json!(false)),
        ),
        (
            "metrics",
            Box::new(|r| r["metrics_summary"]["p_value"] = json!(0.01)),
        ),
        (
            "violations",
            Box::new(|r| r["layer_results"][0]["violations"] = json!(["x"])),
        ),
        (
            "subject",
            Box::new(|r| r["binding"]["subject_sha256"] = json!(OTHER)),
        ),
        (
            "evaluator",
            Box::new(|r| r["binding"]["evaluator_binary_sha256"] = json!(HOLDOUTS)),
        ),
    ];
    for (name, f) in edits {
        let d = fresh(&format!("score-{name}"));
        add_evaluation(&d, &judge(), |_| {}, None);
        edit(&d, "evaluation_receipt", |r| f(r));
        assert_eq!(
            code(&verify_with(&d, &opts(&judge()))),
            "evaluation_signature_bad",
            "{name}"
        );
    }
}

#[test]
fn wrong_keys_are_refused() {
    // Signed by someone else (a proposer's own key), checked against the pinned judge key.
    let d = fresh("wrong-signer");
    add_evaluation(&d, &key(9), |_| {}, None);
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_signature_bad"
    );
    // Signed by the judge, checked against a different pinned key.
    let d = fresh("wrong-pin");
    add_evaluation(&d, &judge(), |_| {}, None);
    assert_eq!(
        code(&verify_with(&d, &opts(&key(11)))),
        "evaluation_signature_bad"
    );
    // A bundle cannot smuggle its own key: nothing in it is read as a key.
    let mut c = companion(&d);
    c["evaluation"]["judge_public_key"] = json!(hex::encode(
        key(11).verifying_key().to_sec1_point(false).as_bytes()
    ));
    write_companion(&d, &c);
    assert_eq!(code(&verify(&d)), "evaluation_untrusted");
}

#[test]
fn a_genuine_receipt_for_other_bytes_is_a_substituted_change() {
    let d = fresh("other-bytes");
    add_evaluation(
        &d,
        &judge(),
        |r| r["binding"]["subject_sha256"] = json!(OTHER),
        None,
    );
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_subject_mismatch"
    );
    let d = fresh("other-path");
    add_evaluation(
        &d,
        &judge(),
        |r| r["binding"]["subject_path"] = json!("test/test_clamp.c"),
        None,
    );
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_subject_mismatch"
    );
}

#[test]
fn altered_policies_are_refused() {
    // The retained policy edited after the evaluation.
    let d = fresh("policy-edited");
    add_evaluation(&d, &judge(), |_| {}, None);
    edit(&d, "evaluation_policy", |p| {
        p["min_holdout_pass_ratio"] = json!(0.5)
    });
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_policy_mismatch"
    );
    // Policies that do not allow the write, or waive admission.
    let (path, _) = subject(&base());
    for (name, mut p) in [
        ("not-allowed", policy("README.md")),
        ("waives", policy(&path)),
        ("ratio", policy(&path)),
    ] {
        match name {
            "waives" => p["require_admitted"] = json!(false),
            "ratio" => p["min_holdout_pass_ratio"] = json!(0.0),
            _ => {}
        }
        let d = fresh(&format!("policy-{name}"));
        add_evaluation(&d, &judge(), |_| {}, Some(p));
        assert_eq!(
            code(&verify_with(&d, &opts(&judge()))),
            "evaluation_policy_mismatch",
            "{name}"
        );
    }
}

#[test]
fn a_holdout_set_other_than_the_pinned_one_is_refused() {
    let d = fresh("holdouts");
    add_evaluation(
        &d,
        &judge(),
        |r| r["binding"]["holdout_set_sha256"] = json!(OTHER),
        None,
    );
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_holdout_mismatch"
    );
}

#[test]
fn below_threshold_or_not_admitted_is_refused() {
    for (name, f) in [
        (
            "short",
            Box::new(|r: &mut Value| r["binding"]["holdouts_passed"] = json!(3))
                as Box<dyn Fn(&mut Value)>,
        ),
        (
            "zero",
            Box::new(|r: &mut Value| {
                r["binding"]["holdouts_passed"] = json!(0);
                r["binding"]["holdouts_total"] = json!(0);
            }),
        ),
        (
            "over",
            Box::new(|r: &mut Value| r["binding"]["holdouts_passed"] = json!(5)),
        ),
        (
            "rejected",
            Box::new(|r: &mut Value| r["admitted"] = json!(false)),
        ),
    ] {
        let d = fresh(&format!("threshold-{name}"));
        add_evaluation(&d, &judge(), |r| f(r), None);
        assert_eq!(
            code(&verify_with(&d, &opts(&judge()))),
            "evaluation_below_threshold",
            "{name}"
        );
    }
}

#[test]
fn malformed_and_unsupported_receipts_are_refused() {
    let d = fresh("malformed");
    add_evaluation(&d, &judge(), |_| {}, None);
    edit(&d, "evaluation_receipt", |r| {
        r.as_object_mut().unwrap().remove("layer_results");
    });
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_malformed"
    );
    let d = fresh("binding");
    add_evaluation(&d, &judge(), |_| {}, None);
    let mut c = companion(&d);
    c["evaluation"]["binding"] = json!("rsi-eval/1");
    write_companion(&d, &c);
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "unsupported_evaluation"
    );
    let d = fresh("version3");
    add_evaluation(&d, &judge(), |r| r["format_version"] = json!(3), None);
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "unsupported_evaluation"
    );
}

#[test]
fn a_receipt_judged_from_another_commit_is_refused() {
    let d = fresh("parent");
    add_evaluation(&d, &judge(), |r| r["parent_id"] = json!("0000000"), None);
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_parent_mismatch"
    );
}

#[test]
fn the_operator_can_pin_the_policy() {
    let d = fresh("pinned");
    add_evaluation(&d, &judge(), |_| {}, None);
    let retained = std::fs::read(path_of(&d, "evaluation_policy")).unwrap();
    let pinned = |sha: String| Options {
        judge_key: Some(*judge().verifying_key()),
        policy_sha256: Some(sha),
    };
    assert_eq!(verify_with(&d, &pinned(sha256_hex(&retained))), baseline());
    // A self-consistent receipt and policy pair, but not the operator's policy.
    assert_eq!(
        code(&verify_with(&d, &pinned(OTHER.to_string()))),
        "evaluation_policy_mismatch"
    );
}

#[test]
fn a_subject_under_a_protected_path_is_refused() {
    let (path, _) = subject(&base());
    let mut p = policy(&path);
    p["protected_paths"] = json!([path.split('/').next().unwrap()]);
    let d = fresh("protected");
    add_evaluation(&d, &judge(), |_| {}, Some(p));
    assert_eq!(
        code(&verify_with(&d, &opts(&judge()))),
        "evaluation_policy_mismatch"
    );
}

// ---------------------------------------------------------------- cross-language fixture

fn cross() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/rsi-eval-v2")
}

fn cross_key() -> VerifyingKey {
    parse_judge_key(&std::fs::read_to_string(cross().join("judge.pub")).unwrap()).unwrap()
}

struct Cross {
    receipt: Vec<u8>,
    policy: Vec<u8>,
    subject: Vec<u8>,
    parent: String,
}

fn cross_files() -> Cross {
    let f = cross();
    let receipt = std::fs::read(f.join("receipt.json")).unwrap();
    let parent = serde_json::from_slice::<Value>(&receipt).unwrap()["parent_id"]
        .as_str()
        .unwrap()
        .to_string();
    Cross {
        receipt,
        policy: std::fs::read(f.join("policy.json")).unwrap(),
        subject: std::fs::read(f.join("subject.md")).unwrap(),
        parent,
    }
}

fn cross_check(
    c: &Cross,
    receipt: &[u8],
    policy: &[u8],
    key: &VerifyingKey,
    path: &str,
    content: &[u8],
) -> Result<(), String> {
    precheck(
        receipt,
        policy,
        key,
        &Expect {
            path,
            content,
            parent_commit: &c.parent,
            policy_sha256: Some(&sha256_hex(&c.policy)),
        },
    )
}

/// A receipt written and signed by spark-rsi's own judge binary (fixtures/rsi-eval-v2/README.md
/// says how it was made) verifies here: the two implementations agree on every digest byte.
#[test]
fn spark_rsi_receipt_verifies_byte_for_byte() {
    let c = cross_files();
    let k = cross_key();
    cross_check(&c, &c.receipt, &c.policy, &k, "README.md", &c.subject).unwrap();

    let refused = |r: Result<(), String>, code: &str| {
        let e = r.unwrap_err();
        assert!(e.starts_with(&format!("FAIL {code}:")), "{e}");
    };
    let mut other = c.subject.clone();
    other.push(b'\n');
    refused(
        cross_check(&c, &c.receipt, &c.policy, &k, "README.md", &other),
        "evaluation_subject_mismatch",
    );
    refused(
        cross_check(&c, &c.receipt, &c.policy, &k, "src/main.rs", &c.subject),
        "evaluation_subject_mismatch",
    );
    refused(
        cross_check(
            &c,
            &c.receipt,
            &c.policy,
            judge().verifying_key(),
            "README.md",
            &c.subject,
        ),
        "evaluation_signature_bad",
    );
    let mut policy2 = c.policy.clone();
    policy2.push(b'\n');
    refused(
        cross_check(&c, &c.receipt, &policy2, &k, "README.md", &c.subject),
        "evaluation_policy_mismatch",
    );
    refused(
        precheck(
            &c.receipt,
            &c.policy,
            &k,
            &Expect {
                path: "README.md",
                content: &c.subject,
                parent_commit: "0000000",
                policy_sha256: None,
            },
        ),
        "evaluation_parent_mismatch",
    );
}

#[test]
fn spark_rsi_receipt_with_any_field_changed_fails() {
    let c = cross_files();
    let receipt: Value = serde_json::from_slice(&c.receipt).unwrap();
    let paths: [&[&str]; 6] = [
        &["cycle_id"],
        &["evaluated_at"],
        &["binding", "holdouts_total"],
        &["binding", "evaluator_binary_sha256"],
        &["layer_results"],
        &["metrics_summary"],
    ];
    for path in paths {
        let mut r = receipt.clone();
        let mut v = &mut r;
        for k in &path[..path.len() - 1] {
            v = &mut v[*k];
        }
        let last = path[path.len() - 1];
        v[last] = match &v[last] {
            Value::String(s) => json!(format!("{s}x")),
            Value::Number(n) => json!(n.as_u64().unwrap_or(0) + 1),
            Value::Array(a) => json!(a[..a.len().saturating_sub(1)]),
            _ => Value::Null,
        };
        let e = cross_check(
            &c,
            &serde_json::to_vec(&r).unwrap(),
            &c.policy,
            &cross_key(),
            "README.md",
            &c.subject,
        )
        .unwrap_err();
        assert!(
            e.starts_with("FAIL evaluation_signature_bad"),
            "{path:?}: {e}"
        );
    }
}
