//! `vac-test-run/1`: the fix-the-test slice's extra evidence, and the `aien.native` claim.
//! Specification: provenance/BINDING.md ("Test run", "Source pin", "Native claim").
//!
//! The daemon only writes the file; it does not run tests. The test run is HARNESS evidence: the
//! harness ran the task's test command before the write and after the daemon's ack, and retained
//! what it saw. This module checks that the retained records agree with each other and with the
//! daemon's own grant and ack. It cannot tell a harness export from a hand-written file (unsigned).
//! It reads and compares; it authorizes and repairs nothing.

use super::{fail, s, Archive, Fail};
use crate::ledger;
use serde_json::Value;
use std::collections::BTreeSet;

pub const BINDING: &str = "vac-test-run/1";
pub const MISSING_NATIVE: &str = "link:native";
const RUN_KIND: &str = "vac-test-run";
const PIN_KIND: &str = "vac-source-pin";

fn is_hex64(x: &str) -> bool {
    x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_hex40(x: &str) -> bool {
    x.len() == 40 && x.bytes().all(|b| b.is_ascii_hexdigit())
}

fn native_of(m: &Value) -> Option<&Value> {
    m.get("aien")
        .and_then(|a| a.get("native"))
        .filter(|n| !n.is_null())
}

/// Missing links implied by a slice bundle: with a `test_run` section, the daemon must have
/// reported a native compose library (`aien.native.claimed: true`), else `link:native` is missing.
pub fn implied_missing(m: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if m.get("test_run").is_some() {
        let claimed = native_of(m).and_then(|n| n.get("claimed")) == Some(&Value::Bool(true));
        if !claimed {
            out.insert(MISSING_NATIVE.to_string());
        }
    }
    out
}

/// True when the bundle carries a test run or the records of one (a task, a source pin).
pub fn claims_test_run(a: &Archive, m: &Value) -> bool {
    m.get("test_run").is_some_and(|v| !v.is_null())
        || ["task", "source_pin", "test_run_record"]
            .iter()
            .any(|n| a.has(n))
}

fn rec_json(a: &Archive, name: &str, code: &str) -> Result<Value, Fail> {
    if !a.has(name) {
        return fail(code, format!("record {name} absent"));
    }
    match a.json(name)? {
        Some(v) => Ok(v),
        None => fail(code, format!("record {name} not retained")),
    }
}

/// Checks for the slice sections. `ledger_checked` is true when the effect link was the strong
/// ledger binding (whose checks already ran).
pub fn check(a: &Archive, m: &Value, ledger_checked: bool) -> Result<(), Fail> {
    let section = m.get("test_run").filter(|v| !v.is_null());
    // A bundle that carries a task or a source pin claims a fix: the test run must be there.
    let claims_fix = ["task", "source_pin", "test_run_record"]
        .iter()
        .any(|n| a.has(n));
    let Some(sec) = section else {
        if claims_fix {
            return fail(
                "test_run_missing",
                "task/source_pin/test_run_record present but no test_run section",
            );
        }
        return check_native(a, m);
    };
    if !ledger_checked {
        return fail(
            "malformed_companion",
            "test_run requires effect.binding aien-ledger-slice/1",
        );
    }
    if s(sec, &["binding"]) != Some(BINDING) {
        return fail(
            "unsupported_test_run",
            format!(
                "test_run.binding={}",
                s(sec, &["binding"]).unwrap_or("absent")
            ),
        );
    }
    let Some(rec_name) = s(sec, &["record"]) else {
        return fail("test_run_missing", "test_run.record");
    };
    let run = rec_json(a, rec_name, "test_run_missing")?;
    if s(&run, &["kind"]) != Some(RUN_KIND) || run.get("v").and_then(Value::as_u64) != Some(1) {
        return fail("unsupported_test_run", "record kind/v");
    }
    let task = rec_json(a, "task", "test_run_missing")?;
    let facts = ledger::facts(a)?;

    // Task scope: the task names the one file it allows to change, and the grant must write that
    // file. Without it a run that edited the test instead of the code would still verify.
    match s(&task, &["target_path"]) {
        Some(t) if t == facts.path => {}
        Some(t) => {
            return fail(
                "task_scope_mismatch",
                format!("task.target_path={t} vs ledger_grant.path={}", facts.path),
            )
        }
        None => {
            return fail(
                "task_scope_mismatch",
                "task.target_path absent: the task does not say which file may change",
            )
        }
    }
    let pin = rec_json(a, "source_pin", "source_pin_mismatch")?;

    // Source pin: the commit the task starts from, and the exact target bytes the grant replaced.
    let pin_bad = |what: &str| -> Result<(), Fail> { fail("source_pin_mismatch", what) };
    if s(&pin, &["kind"]) != Some(PIN_KIND) {
        return pin_bad("source_pin.kind");
    }
    if s(&pin, &["target_path"]) != Some(facts.path.as_str()) {
        return pin_bad("source_pin.target_path vs ledger_grant.path");
    }
    match (
        s(&pin, &["target_blob_sha256"]),
        facts.prior_sha256.as_deref(),
    ) {
        (Some(p), Some(g)) if is_hex64(p) && p == g => {}
        _ => return pin_bad("source_pin.target_blob_sha256 vs ledger_grant.prior_sha256"),
    }
    if s(&pin, &["commit"]).is_none() || s(&pin, &["commit"]) != s(&task, &["repo_commit"]) {
        return pin_bad("source_pin.commit vs task.repo_commit");
    }

    // The test run.
    let bad = |what: &str| -> Result<(), Fail> { fail("test_run_mismatch", what) };
    let tid = s(&task, &["task_id"]);
    if tid.is_none() || s(sec, &["task_id"]) != tid || s(&run, &["task_id"]) != tid {
        return bad("task_id (section, record and task must agree)");
    }
    if run.get("argv") != task.get("test_cmd") || !run.get("argv").is_some_and(Value::is_array) {
        return bad("argv vs task.test_cmd");
    }
    match run.get("exit_code").and_then(Value::as_i64) {
        Some(0) => {}
        Some(c) => return bad(&format!("exit_code={c} (a fix must pass: exit 0)")),
        None => return bad("exit_code"),
    }
    match run.get("test_exit_before").and_then(Value::as_i64) {
        Some(0) => return bad("test_exit_before=0 (the test already passed: nothing was fixed)"),
        Some(_) => {}
        None => return bad("test_exit_before"),
    }
    match s(&run, &["target_blob_sha256_after"]) {
        Some(b) if is_hex64(b) && b == facts.disk_sha256 && b == facts.content_sha256 => {}
        _ => return bad("target_blob_sha256_after vs ledger_ack.disk_sha256"),
    }
    for (field, record) in [
        ("stdout_sha256", "test_stdout"),
        ("stderr_sha256", "test_stderr"),
    ] {
        let Some(d) = s(&run, &[field]).filter(|d| is_hex64(d)) else {
            return bad(field);
        };
        // A claimed digest must be backed by retained bytes that hash to it. A missing entry, or
        // one labelled not retained, would otherwise let any 64-hex value pass.
        if !a.has(record) {
            return bad(&format!(
                "{record} record absent: {field} has nothing to match"
            ));
        }
        match a.bytes(record)? {
            None => return bad(&format!("{record} not retained: {field} cannot be checked")),
            Some(_) if a.sha(record)? != d => return bad(&format!("{field} vs retained {record}")),
            Some(_) => {}
        }
    }
    check_native(a, m)
}

/// `aien.native`: the claim that the daemon's compose library was the native one, checked against
/// the retained `ComposeRecall` report (`compose_recall`).
fn check_native(a: &Archive, m: &Value) -> Result<(), Fail> {
    let Some(n) = native_of(m) else {
        return Ok(());
    };
    let (Some(claimed), Some(sha)) = (
        n.get("claimed").and_then(Value::as_bool),
        s(n, &["omega_sha"]),
    ) else {
        return fail("malformed_companion", "aien.native.claimed/omega_sha");
    };
    let recall = if a.has("compose_recall") {
        a.json("compose_recall")?
    } else {
        None
    };
    let recall_native = recall
        .as_ref()
        .and_then(|r| r.get("compose_native"))
        .and_then(Value::as_bool);
    let recall_sha = recall.as_ref().and_then(|r| s(r, &["omega_sha"]));
    if claimed {
        match (recall_native, recall_sha) {
            (Some(true), Some(r)) if is_hex40(r) && r == sha && is_hex40(sha) => {}
            (None, _) => {
                return fail(
                    "native_claim_contradicted",
                    "claimed native but no compose_recall report with compose_native",
                )
            }
            (Some(false), _) => {
                return fail(
                    "native_claim_contradicted",
                    "claimed native but compose_recall.compose_native=false",
                )
            }
            _ => {
                return fail(
                    "native_claim_contradicted",
                    "aien.native.omega_sha vs compose_recall.omega_sha",
                )
            }
        }
    } else if recall_native == Some(true) {
        return fail(
            "native_claim_contradicted",
            "claimed:false but compose_recall.compose_native=true",
        );
    }
    Ok(())
}
