//! `rsi-eval/2`: the independent judge's signed evaluation of the exact change the daemon wrote.
//! Specification: provenance/BINDING.md ("Evaluation") and spark-rsi docs/RECEIPT-V2.md.
//!
//! The invariant: no change may be promoted unless its exact identity matches the identity covered
//! by a valid, independently signed evaluation. This module recomputes the judge receipt's
//! version 2 digest, checks its P-256 signature under a judge key supplied by the operator out of
//! band (never read from the bundle), and requires the evaluated subject to be the bytes the
//! daemon granted and read back from disk. It reads and compares; it authorizes nothing.

use super::{fail, s, Archive, Fail};
use crate::ledger;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::Signature;
pub use p256::ecdsa::VerifyingKey;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const BINDING: &str = "rsi-eval/2";
const RECEIPT_DOMAIN: &str = "spark-rsi.evaluation-receipt.v2";
const RECEIPT_RECORD: &str = "evaluation_receipt";
const POLICY_RECORD: &str = "evaluation_policy";

/// Parses the operator's judge key file contents: the SEC1 public key in hex.
pub fn parse_judge_key(text: &str) -> Result<VerifyingKey, String> {
    let bytes = hex::decode(text.trim()).map_err(|e| format!("judge key is not hex: {e}"))?;
    VerifyingKey::from_sec1_bytes(&bytes)
        .map_err(|e| format!("judge key is not a P-256 point: {e}"))
}

fn is_hex64(x: &str) -> bool {
    x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True when the bundle carries the records of a judge evaluation.
pub(crate) fn claims_evaluation(a: &Archive, m: &Value) -> bool {
    m.get("evaluation").is_some_and(|v| !v.is_null())
        || [RECEIPT_RECORD, POLICY_RECORD].iter().any(|n| a.has(n))
}

fn put(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// Field readers that refuse rather than default: a missing field is a malformed receipt.
struct R<'a>(&'a Value);

impl R<'_> {
    fn str(&self, path: &[&str]) -> Result<&str, Fail> {
        s(self.0, path).map_or_else(
            || {
                fail(
                    "evaluation_malformed",
                    format!("receipt.{}", path.join(".")),
                )
            },
            Ok,
        )
    }
    fn bool(&self, path: &[&str]) -> Result<bool, Fail> {
        let mut v = self.0;
        for k in path {
            v = match v.get(k) {
                Some(x) => x,
                None => {
                    return fail(
                        "evaluation_malformed",
                        format!("receipt.{}", path.join(".")),
                    )
                }
            };
        }
        v.as_bool().map_or_else(
            || {
                fail(
                    "evaluation_malformed",
                    format!("receipt.{}", path.join(".")),
                )
            },
            Ok,
        )
    }
}

fn num_u64(v: &Value, what: &str) -> Result<u64, Fail> {
    v.as_u64()
        .map_or_else(|| fail("evaluation_malformed", what), Ok)
}

fn num_f64(v: &Value, what: &str) -> Result<f64, Fail> {
    v.as_f64()
        .map_or_else(|| fail("evaluation_malformed", what), Ok)
}

/// Recomputes the version 2 receipt digest (RECEIPT-V2.md "Digest") from the parsed receipt.
pub fn receipt_digest_v2(r: &Value) -> Result<String, Fail> {
    let g = R(r);
    let mut h = Sha256::new();
    put(&mut h, RECEIPT_DOMAIN.as_bytes());
    for k in [
        "cycle_id",
        "candidate_id",
        "parent_id",
        "evaluated_at",
        "evaluator_version",
    ] {
        put(&mut h, g.str(&[k])?.as_bytes());
    }
    for k in [
        "evaluator_binary_sha256",
        "subject_path",
        "subject_sha256",
        "holdout_set_sha256",
        "policy_sha256",
    ] {
        put(&mut h, g.str(&["binding", k])?.as_bytes());
    }
    let b = r.get("binding").unwrap_or(&Value::Null);
    for k in ["holdouts_total", "holdouts_passed"] {
        let n = num_u64(b.get(k).unwrap_or(&Value::Null), k)?;
        put(&mut h, &n.to_le_bytes());
    }
    for k in [
        "admitted",
        "passed_all_hard_invariants",
        "passed_statistical_gates",
    ] {
        put(&mut h, &[g.bool(&[k])? as u8]);
    }
    let Some(layers) = r.get("layer_results").and_then(Value::as_array) else {
        return fail("evaluation_malformed", "receipt.layer_results");
    };
    put(&mut h, &(layers.len() as u64).to_le_bytes());
    for l in layers {
        let lg = R(l);
        put(&mut h, lg.str(&["layer_name"])?.as_bytes());
        put(&mut h, &[lg.bool(&["is_hard_invariant"])? as u8]);
        put(&mut h, &[lg.bool(&["passed"])? as u8]);
        let score = num_f64(l.get("score").unwrap_or(&Value::Null), "layer.score")?;
        put(&mut h, &score.to_bits().to_le_bytes());
        put(&mut h, lg.str(&["summary"])?.as_bytes());
        let Some(vs) = l.get("violations").and_then(Value::as_array) else {
            return fail("evaluation_malformed", "layer.violations");
        };
        put(&mut h, &(vs.len() as u64).to_le_bytes());
        for v in vs {
            let Some(v) = v.as_str() else {
                return fail("evaluation_malformed", "layer.violations[]");
            };
            put(&mut h, v.as_bytes());
        }
    }
    match r.get("metrics_summary") {
        None | Some(Value::Null) => put(&mut h, &[0u8]),
        Some(m) => {
            put(&mut h, &[1u8]);
            for k in [
                "latency_delta_pct",
                "p_value",
                "p95_ci_upper_degradation_pct",
                "p99_ci_upper_degradation_pct",
                "rss_growth_pct",
            ] {
                let x = num_f64(m.get(k).unwrap_or(&Value::Null), k)?;
                put(&mut h, &x.to_bits().to_le_bytes());
            }
            let mb = num_u64(
                m.get("candidate_resident_mb").unwrap_or(&Value::Null),
                "candidate_resident_mb",
            )?;
            put(&mut h, &mb.to_le_bytes());
        }
    }
    Ok(hex::encode(h.finalize()))
}

fn verify_signature(r: &Value, digest_hex: &str, key: &VerifyingKey) -> Result<(), Fail> {
    let bad = |w: &str| fail("evaluation_signature_bad", w);
    let Some(sig) = s(r, &["signature"]) else {
        return bad("receipt.signature absent");
    };
    let raw = sig.strip_prefix("tpm2-p256:").unwrap_or(sig);
    let Ok(sig_bytes) = hex::decode(raw) else {
        return bad("signature is not hex");
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return bad("signature is not a P-256 signature");
    };
    let Ok(digest) = hex::decode(digest_hex) else {
        return bad("digest");
    };
    // The judge signs the 32 raw digest bytes as its message (ECDSA with SHA-256 over them).
    if key.verify(&digest, &sig).is_err() {
        return bad("signature does not verify under the operator-supplied judge key");
    }
    Ok(())
}

/// Checks the `evaluation` section. `judge_key` comes from the operator, never from the bundle.
/// `ledger_checked` is true when the effect link was the strong ledger binding.
pub(crate) fn check(
    a: &Archive,
    m: &Value,
    ledger_checked: bool,
    opts: &crate::Options,
) -> Result<(), Fail> {
    let judge_key = opts.judge_key.as_ref();
    let Some(sec) = m.get("evaluation").filter(|v| !v.is_null()) else {
        if claims_evaluation(a, m) {
            return fail(
                "evaluation_missing",
                "evaluation records present but no evaluation section",
            );
        }
        if judge_key.is_some() {
            return fail(
                "evaluation_missing",
                "a judge key was supplied but the bundle carries no evaluation",
            );
        }
        return Ok(());
    };
    if !ledger_checked {
        return fail(
            "malformed_companion",
            "evaluation requires effect.binding aien-ledger-slice/1",
        );
    }
    match s(sec, &["binding"]) {
        Some(BINDING) => {}
        other => {
            return fail(
                "unsupported_evaluation",
                format!("evaluation.binding={}", other.unwrap_or("absent")),
            )
        }
    }
    let load = |name: &str| -> Result<Value, Fail> {
        if !a.has(name) {
            return fail("evaluation_missing", format!("record {name} absent"));
        }
        match a.json(name)? {
            Some(v) => Ok(v),
            None => fail("evaluation_missing", format!("record {name} not retained")),
        }
    };
    let r = load(RECEIPT_RECORD)?;
    // The policy the judge ran under must be retained byte for byte.
    if !a.has(POLICY_RECORD) || a.bytes(POLICY_RECORD)?.is_none() {
        return fail(
            "evaluation_missing",
            "record evaluation_policy absent or not retained",
        );
    }
    let policy_sha = a.sha(POLICY_RECORD)?.to_string();
    let policy = load(POLICY_RECORD)?;
    // The evaluated subject is the change the daemon granted and read back, judged from the
    // commit the task's source pin names.
    let facts = ledger::facts(a)?;
    let pin = if a.has("source_pin") {
        a.json("source_pin")?
    } else {
        None
    };
    let Some(parent) = pin.as_ref().and_then(|p| s(p, &["commit"])) else {
        return fail(
            "evaluation_parent_mismatch",
            "no retained source_pin commit to bind the judged parent to",
        );
    };
    core(
        &r,
        &policy_sha,
        &policy,
        judge_key,
        &Want {
            path: &facts.path,
            content_sha256: &facts.content_sha256,
            disk_sha256: &facts.disk_sha256,
            parent_commit: parent,
            pinned_policy_sha256: opts.policy_sha256.as_deref(),
        },
    )
}

/// What the harness is about to propose: the file, its whole new bytes, the commit the workspace
/// is at (the judged parent), and the operator's pinned policy digest when it has one.
pub struct Expect<'a> {
    pub path: &'a str,
    pub content: &'a [u8],
    pub parent_commit: &'a str,
    pub policy_sha256: Option<&'a str>,
}

/// What a receipt must match, from the audit (ledger facts) or from the harness (before a write).
struct Want<'a> {
    path: &'a str,
    content_sha256: &'a str,
    disk_sha256: &'a str,
    parent_commit: &'a str,
    pinned_policy_sha256: Option<&'a str>,
}

/// The harness gate before the write: the same checks as the audit, run on the judge's receipt
/// and policy bytes against exactly what is about to be proposed. Returns the refusal line on
/// failure. Nothing reaches the daemon unless this passes.
pub fn precheck(
    receipt: &[u8],
    policy: &[u8],
    judge_key: &VerifyingKey,
    want: &Expect,
) -> Result<(), String> {
    let r = crate::strict::parse(receipt)
        .ok_or("FAIL evaluation_malformed: receipt is not strict JSON")?;
    let p = crate::strict::parse(policy)
        .ok_or("FAIL evaluation_malformed: policy is not strict JSON")?;
    let content = hex::encode(Sha256::digest(want.content));
    core(
        &r,
        &hex::encode(Sha256::digest(policy)),
        &p,
        Some(judge_key),
        &Want {
            path: want.path,
            content_sha256: &content,
            disk_sha256: &content,
            parent_commit: want.parent_commit,
            pinned_policy_sha256: want.policy_sha256,
        },
    )
    .map_err(|Fail(line)| line)
}

/// Every check on a parsed receipt and policy. `w.content_sha256` is what the change carries and
/// `w.disk_sha256` what was read back after the write (the same value before the write).
fn core(
    r: &Value,
    policy_sha256: &str,
    policy: &Value,
    judge_key: Option<&VerifyingKey>,
    w: &Want,
) -> Result<(), Fail> {
    let (path, content_sha256, disk_sha256) = (w.path, w.content_sha256, w.disk_sha256);
    match r.get("format_version").and_then(Value::as_u64) {
        None | Some(1) => {
            return fail(
                "evaluation_v1_insufficient",
                "a version 1 receipt does not bind the evaluated change",
            )
        }
        Some(2) => {}
        Some(v) => return fail("unsupported_evaluation", format!("format_version={v}")),
    }
    let digest = receipt_digest_v2(r)?;
    if s(r, &["receipt_digest"]) != Some(digest.as_str()) {
        return fail(
            "evaluation_signature_bad",
            "receipt_digest does not match the receipt's fields",
        );
    }
    let Some(key) = judge_key else {
        return fail(
            "evaluation_untrusted",
            "no judge public key supplied; a receipt never carries the key that verifies it",
        );
    };
    verify_signature(r, &digest, key)?;

    let g = R(r);
    let subject_path = g.str(&["binding", "subject_path"])?;
    if subject_path != path {
        return fail(
            "evaluation_subject_mismatch",
            format!("binding.subject_path={subject_path} vs ledger_grant.path={path}"),
        );
    }
    let subject = g.str(&["binding", "subject_sha256"])?;
    if !is_hex64(subject) || subject != content_sha256 || subject != disk_sha256 {
        return fail(
            "evaluation_subject_mismatch",
            "binding.subject_sha256 vs ledger_grant.content_sha256 and ledger_ack.disk_sha256",
        );
    }
    // The tree the judge compared against is the commit the change was made from.
    let parent = g.str(&["parent_id"])?;
    if parent != w.parent_commit {
        return fail(
            "evaluation_parent_mismatch",
            format!(
                "parent_id={parent} vs source_pin.commit={}",
                w.parent_commit
            ),
        );
    }

    // The policy the judge ran under is the retained policy, and it pins the holdout set.
    if policy_sha256 != g.str(&["binding", "policy_sha256"])? {
        return fail(
            "evaluation_policy_mismatch",
            "binding.policy_sha256 vs retained evaluation_policy",
        );
    }
    // When the operator pins a policy, the judge must have run under exactly that one.
    if let Some(pinned) = w.pinned_policy_sha256 {
        if policy_sha256 != pinned {
            return fail(
                "evaluation_policy_mismatch",
                "retained evaluation_policy is not the operator's pinned policy",
            );
        }
    }
    let protected = policy
        .get("protected_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|p| {
            let p = p.trim_end_matches('/');
            path == p || path.starts_with(&format!("{p}/"))
        });
    if let Some(p) = protected {
        return fail(
            "evaluation_policy_mismatch",
            format!("{path} is under protected path {p} in evaluation_policy"),
        );
    }
    let allowed = policy
        .get("allowed_targets")
        .and_then(Value::as_array)
        .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(path)));
    if !allowed {
        return fail(
            "evaluation_policy_mismatch",
            format!("{path} is not an allowed target in evaluation_policy"),
        );
    }
    if policy.get("require_admitted") != Some(&Value::Bool(true)) {
        return fail(
            "evaluation_policy_mismatch",
            "evaluation_policy.require_admitted must be true",
        );
    }
    let Some(ratio) = policy
        .get("min_holdout_pass_ratio")
        .and_then(Value::as_f64)
        .filter(|x| *x > 0.0 && *x <= 1.0)
    else {
        return fail(
            "evaluation_policy_mismatch",
            "evaluation_policy.min_holdout_pass_ratio",
        );
    };
    if s(policy, &["holdout_set_sha256"]) != Some(g.str(&["binding", "holdout_set_sha256"])?) {
        return fail(
            "evaluation_holdout_mismatch",
            "binding.holdout_set_sha256 vs evaluation_policy.holdout_set_sha256",
        );
    }
    let b = r.get("binding").unwrap_or(&Value::Null);
    let total = num_u64(
        b.get("holdouts_total").unwrap_or(&Value::Null),
        "holdouts_total",
    )?;
    let passed = num_u64(
        b.get("holdouts_passed").unwrap_or(&Value::Null),
        "holdouts_passed",
    )?;
    if !g.bool(&["admitted"])? {
        return fail(
            "evaluation_below_threshold",
            "the judge did not admit the change",
        );
    }
    if total == 0 || passed > total || (passed as f64) < ratio * (total as f64) {
        return fail(
            "evaluation_below_threshold",
            format!("holdouts passed {passed}/{total} below policy ratio {ratio}"),
        );
    }
    Ok(())
}
