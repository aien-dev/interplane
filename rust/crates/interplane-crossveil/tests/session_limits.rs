//! Session state limits and close_trace (spec/CORE.md, "Session state limits").
//!
//! The same scenarios, code strings and message strings are tested in
//! python/tests/test_session_limits.py.
use interplane_core::*;
use interplane_crossveil::*;
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Value};

const NOW: &str = "2026-01-01T00:00:00Z";
const LIMIT: &str = "session_limit_exceeded";

fn pipe<'a>(rt: &'a mut dyn RuntimeAuthority, limits: Limits) -> Pipeline<'a> {
    Pipeline::new(
        DialectRegistry::with_defaults(),
        mock_mapping_table(),
        rt,
        limits,
        RequestLedger::new(),
    )
}

fn env(trace: &str, message_id: &str, request_id: &str) -> Value {
    json!({
        "interplane_version": "0.1", "message_id": message_id, "trace_id": trace,
        "parent_id": null, "timestamp": NOW,
        "source": {"kind": "model", "id": "m"}, "destination": {"kind": "runtime", "id": "mock"},
        "payload": {
            "kind": "tool_request", "request_id": request_id,
            "tool": {"namespace": null, "name": "read_file"},
            "arguments": {"path": "/tmp/a"},
            "provenance": {"dialect": "openai", "parser_version": "1.0.0"}
        }
    })
}

/// `(status, code, message)` of one admitted envelope.
fn admit(
    p: &mut Pipeline<'_>,
    trace: &str,
    message_id: &str,
    request_id: &str,
) -> (String, Option<String>, Option<String>) {
    let (res, _) = p.admit_value(&env(trace, message_id, request_id));
    let status = res.status.as_str().to_string();
    match res.error {
        None => (status, None, None),
        Some(e) => (status, Some(e.code.as_str().to_string()), Some(e.message)),
    }
}

fn refused(code: &str, message: &str) -> (String, Option<String>, Option<String>) {
    (
        "rejected".to_string(),
        Some(code.to_string()),
        Some(message.to_string()),
    )
}

fn ok() -> (String, Option<String>, Option<String>) {
    ("ok".to_string(), None, None)
}

fn input_rec(id: &str, trace: &str) -> InputRecord {
    serde_json::from_value(json!({
        "input_id": id, "content_kind": "user_request", "trust": "user_supplied",
        "source": {"kind": "operator", "id": "op"}, "origin": "user:prompt",
        "content_digest": format!("sha256:{}", "a".repeat(64)),
        "trace_id": trace, "parent_id": null, "derived_from": []
    }))
    .unwrap()
}

fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}})
}
fn turn(calls: Vec<Value>) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": calls})
}

fn continuation(rid: &str, aid: &str) -> Decision {
    serde_json::from_value(json!({
        "kind": "decision", "request_id": rid, "decision": "authorized",
        "capability": "delete_file",
        "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
        "approval": {"approval_id": aid}
    }))
    .expect("decision")
}

#[test]
fn defaults() {
    let l = Limits::default();
    assert_eq!((l.max_traces, l.max_messages_per_trace), (1024, 4096));
    assert_eq!(
        (l.max_requests_per_trace, l.max_inputs_per_trace),
        (4096, 4096)
    );
    let l: Limits = serde_json::from_str(r#"{"max_traces": 7}"#).unwrap();
    assert_eq!((l.max_traces, l.max_inputs_per_trace), (7, 4096));
    assert_eq!(ErrorCode::SessionLimitExceeded.as_str(), LIMIT);
    assert_eq!(ErrorCode::SessionClosed.as_str(), "session_closed");
}

#[test]
fn request_limit_rejects_before_runtime_contact() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(
        &mut rt,
        Limits {
            max_requests_per_trace: 2,
            ..Limits::default()
        },
    );
    assert_eq!(admit(&mut p, "t", "m1", "r1"), ok());
    assert_eq!(admit(&mut p, "t", "m2", "r2"), ok());
    let got = admit(&mut p, "t", "m3", "r3");
    assert_eq!(
        got,
        refused(LIMIT, "session limit exceeded: max_requests_per_trace")
    );
    assert!(!p.ledger.has_request("t", "r3"));
    assert_eq!(
        admit(&mut p, "t", "m4", "r1").1.as_deref(),
        Some("duplicate_request_id")
    );
    drop(p);
    assert_eq!(rt.decide_calls, 2);
}

#[test]
fn replay_is_still_reported_at_the_message_limit() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(
        &mut rt,
        Limits {
            max_messages_per_trace: 2,
            ..Limits::default()
        },
    );
    assert_eq!(admit(&mut p, "t", "m1", "r1"), ok());
    assert_eq!(admit(&mut p, "t", "m2", "r2"), ok());
    assert_eq!(
        admit(&mut p, "t", "m1", "r9"),
        refused("replayed_message", "replayed message_id: m1")
    );
}

#[test]
fn message_limit() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(
        &mut rt,
        Limits {
            max_messages_per_trace: 2,
            ..Limits::default()
        },
    );
    assert_eq!(admit(&mut p, "t", "m1", "r1"), ok());
    assert_eq!(admit(&mut p, "t", "m2", "r2"), ok());
    assert_eq!(
        admit(&mut p, "t", "m3", "r3"),
        refused(LIMIT, "session limit exceeded: max_messages_per_trace")
    );
    assert!(!p.ledger.has_message("t", "m3"));
    assert_eq!(admit(&mut p, "u", "m1", "r1"), ok());
}

#[test]
fn trace_limit_and_close_frees_a_slot() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(
        &mut rt,
        Limits {
            max_traces: 2,
            ..Limits::default()
        },
    );
    assert_eq!(admit(&mut p, "a", "m1", "r1"), ok());
    assert_eq!(admit(&mut p, "b", "m1", "r1"), ok());
    let n = p.events.len();
    assert_eq!(
        admit(&mut p, "c", "m1", "r1"),
        refused(LIMIT, "session limit exceeded: max_traces")
    );
    assert!(!p.ledger.holds("c") && p.events.len() == n);
    assert_eq!(admit(&mut p, "a", "m2", "r2"), ok());
    p.close_trace("a");
    assert_eq!(admit(&mut p, "c", "m1", "r1"), ok());
}

#[test]
fn input_limit_records_nothing() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(
        &mut rt,
        Limits {
            max_inputs_per_trace: 1,
            ..Limits::default()
        },
    );
    p.register_input(input_rec("i1", "t")).unwrap();
    assert_eq!(
        p.register_input(input_rec("i2", "t")).unwrap_err(),
        "session limit exceeded: max_inputs_per_trace"
    );
    let ids: Vec<&str> = p.inputs("t").iter().map(|r| r.input_id.as_str()).collect();
    assert_eq!(ids, ["i1"]);
    assert_eq!(
        p.register_input(input_rec("i1", "t")).unwrap_err(),
        "duplicate input_id: i1"
    );
}

#[test]
fn close_trace() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, Limits::default());
    assert_eq!(admit(&mut p, "t", "m1", "r1"), ok());
    p.register_input(input_rec("i1", "t")).unwrap();
    p.run_turn(
        "openai",
        "m",
        &turn(vec![call("c1", "delete_file", json!({"path": "/tmp/x"}))]),
        "t",
        0,
    );
    let pa = p.pending_approval("t", "c1").expect("pending");
    let n_t = p.events.len();
    assert_eq!(admit(&mut p, "u", "m1", "r1"), ok());
    let n_u = p.events.len() - n_t;
    assert!(n_t > 0 && n_u > 0);

    p.close_trace("t");
    p.close_trace("t");
    p.close_trace("never-seen");
    assert_eq!(p.events.len(), n_u);
    assert!(p.inputs("t").is_empty() && p.pending_approval("t", "c1").is_none());
    assert_eq!(p.events[0].seq, 0, "only u's events remain");

    let closed = refused("session_closed", "session closed: t");
    assert_eq!(admit(&mut p, "t", "m9", "r9"), closed);
    assert_eq!(admit(&mut p, "t", "m1", "r1"), closed);
    assert_eq!(
        p.register_input(input_rec("i2", "t")).unwrap_err(),
        "session closed: t"
    );
    let d = continuation("c1", &pa.approval_id);
    let err = p
        .continue_approval("t", "c1", &d, &pa.request_digest, NOW)
        .unwrap_err();
    assert_eq!(err.as_str(), "session_closed");
    assert_eq!(
        p.cancel_approval("t", "c1").unwrap_err().as_str(),
        "session_closed"
    );
    assert!(p.inputs("t").is_empty() && p.events.len() == n_u);
    assert_eq!(admit(&mut p, "u", "m2", "r2"), ok());
    assert_eq!(p.events.len(), 2 * n_u);
    let out = p.run_turn(
        "openai",
        "m",
        &turn(vec![call("c2", "read_file", json!({"path": "/tmp/x"}))]),
        "t",
        1,
    );
    let e = out.results[0].error.as_ref().expect("error");
    assert_eq!(e.code.as_str(), "session_closed");
    assert_eq!(e.message, "session closed: t");
    assert!(!p.ledger.holds("t") && p.inputs("t").is_empty());
    assert_eq!(p.events.len(), 2 * n_u, "the closed trace added no events");
}
