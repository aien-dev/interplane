//! Single-use approvals (aien-mcp at 6554aac) through the adapter. The `Pipeline` has no way to
//! carry a grant, so these drive `AienAuthority` through the `RuntimeAuthority` trait directly,
//! the same two calls the pipeline makes (`decide`, then `execute`).
use interplane_adapter_aien::*;
use interplane_core::*;
use interplane_crossveil::{CallContext, RuntimeAuthority};
use serde_json::json;

fn req(id: &str, path: &str, content: &str) -> CapabilityRequest {
    serde_json::from_value(json!({
        "kind": "capability_request", "request_id": id, "runtime": RUNTIME_ID,
        "capability": "write_file",
        "arguments": {"path": path, "content": content},
        "tool": {"namespace": null, "name": "write_file"},
        "mapping": {"table_version": "1", "rule_id": "passthrough:write_file", "passthrough": true}
    }))
    .unwrap()
}

fn ctx() -> CallContext {
    CallContext {
        trace_id: "t".into(),
        message_id: "m".into(),
        parent_id: None,
        session: None,
        exposure: None,
        session: None,
    }
}

/// decide + execute, as the pipeline does (execute only after an authorized decision).
fn run(rt: &mut AienAuthority, r: &CapabilityRequest) -> (Decision, Option<ToolResult>) {
    let d = rt.decide(r, &ctx());
    let res = d.is_authorized().then(|| rt.execute(r, &d, &ctx()));
    (d, res)
}

fn ws() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn without_a_grant_it_stays_pending() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let (dec, res) = run(&mut rt, &req("r1", "a.txt", "x"));
    assert_eq!(dec.decision, DecisionKind::RequiresApproval);
    assert!(res.is_none());
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn approved_effect_executes_once() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let r = req("r1", "a.txt", "hello");
    let grant = rt.issue_approval(&r, 100).unwrap();
    rt.present_approval("r1", grant, 10);
    let (dec, res) = run(&mut rt, &r);
    assert!(dec.is_authorized(), "{dec:?}");
    let res = res.unwrap();
    assert_eq!(res.status, ResultStatus::Ok, "{res:?}");
    assert_eq!(res.data["receipt"]["tool_name"], "write_file");
    assert_eq!(
        std::fs::read_to_string(d.path().join("a.txt")).unwrap(),
        "hello"
    );
    assert_eq!(rt.wire_calls(), 1);
}

#[test]
fn spent_grant_used_again_is_consumed_and_runs_nothing() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let r = req("r1", "a.txt", "hello");
    let grant = rt.issue_approval(&r, 100).unwrap();
    rt.present_approval("r1", grant.clone(), 10);
    // First decide spends the grant (AIEN mints the effect) but execute is never called, so the
    // ledger has no receipt and a second decide is not a replay.
    assert!(rt.decide(&r, &ctx()).is_authorized());
    let (dec, res) = run(&mut rt, &r);
    assert_eq!(dec.decision, DecisionKind::Denied);
    assert!(
        dec.reason.as_deref().unwrap().contains("Consumed"),
        "{dec:?}"
    );
    assert!(res.is_none());
    // The same grant on another request id is a different effect: refused, nothing runs.
    let other = req("r2", "a.txt", "hello");
    let (dec, _) = run_with(&mut rt, &other, grant);
    assert_eq!(dec.decision, DecisionKind::Denied);
    assert_eq!(rt.wire_calls(), 0, "no effect executed");
}

#[test]
fn replay_of_the_same_request_returns_the_existing_receipt() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let r = req("r1", "a.txt", "hello");
    let grant = rt.issue_approval(&r, 100).unwrap();
    rt.present_approval("r1", grant, 10);
    let first = run(&mut rt, &r).1.unwrap();
    assert_eq!(first.status, ResultStatus::Ok);
    // The file is removed so a second real write would be visible.
    std::fs::remove_file(d.path().join("a.txt")).unwrap();
    let (dec, second) = run(&mut rt, &r);
    assert!(dec.is_authorized(), "{dec:?}");
    let second = second.unwrap();
    assert_eq!(second.status, ResultStatus::Ok, "{second:?}");
    assert_eq!(second.data["receipt"], first.data["receipt"]);
    assert_eq!(second.data["output"], first.data["output"]);
    assert_eq!(rt.wire_calls(), 1, "replay did not touch the provider");
    assert!(!d.path().join("a.txt").exists());
}

#[test]
fn grant_for_a_different_effect_is_a_mismatch() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let approved = req("r1", "a.txt", "approved content");
    let grant = rt.issue_approval(&approved, 100).unwrap();
    // Same request id, different arguments: a different intent digest.
    let other = req("r1", "a.txt", "something else");
    rt.present_approval("r1", grant.clone(), 10);
    let (dec, res) = run(&mut rt, &other);
    assert_eq!(dec.decision, DecisionKind::Denied);
    assert!(
        dec.reason.as_deref().unwrap().contains("Mismatch"),
        "{dec:?}"
    );
    assert!(res.is_none());
    assert_eq!(rt.wire_calls(), 0);
    // Nothing was spent: the grant still works for the effect it names.
    let (dec, res) = run_with(&mut rt, &approved, grant);
    assert!(dec.is_authorized(), "{dec:?}");
    assert_eq!(res.unwrap().status, ResultStatus::Ok);
    assert_eq!(rt.wire_calls(), 1);
}

fn run_with(
    rt: &mut AienAuthority,
    r: &CapabilityRequest,
    g: aien_mcp::ApprovalGrant,
) -> (Decision, Option<ToolResult>) {
    rt.present_approval(r.request_id.as_str(), g, 10);
    run(rt, r)
}

#[test]
fn expired_grant_is_refused() {
    let d = ws();
    let mut rt = AienAuthority::new(d.path()).unwrap();
    let r = req("r1", "a.txt", "x");
    let grant = rt.issue_approval(&r, 50).unwrap();
    rt.present_approval("r1", grant.clone(), 50); // now >= expires_at
    let (dec, res) = run(&mut rt, &r);
    assert_eq!(dec.decision, DecisionKind::Denied);
    assert!(
        dec.reason.as_deref().unwrap().contains("Expired"),
        "{dec:?}"
    );
    assert!(res.is_none());
    assert_eq!(rt.wire_calls(), 0);
    // Expiry does not spend the grant: presented in time, it works.
    let (dec, _) = run_with(&mut rt, &r, grant);
    assert!(dec.is_authorized(), "{dec:?}");
}
