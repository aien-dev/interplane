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
    // Inputs cannot open a further trace either.
    assert_eq!(
        p.register_input(input_rec("i1", "c")),
        Err("session limit exceeded: max_traces".to_string())
    );
    assert!(p.inputs("c").is_empty());
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

/// A malformed call's result cites its request id, so no later call in the trace may reuse it
/// (same in both SDKs).
#[test]
fn refused_call_uses_its_request_id() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, Limits::default());
    let bad = json!({"id": "c8", "type": "function", "function": {"name": "read_file", "arguments": "{bad"}});
    let good = call("c8", "read_file", json!({"path": "/tmp/a"}));
    let out = p.run_turn("openai", "m", &turn(vec![bad, good.clone()]), "t", 0);
    let codes: Vec<&str> = out
        .results
        .iter()
        .map(|r| r.error.as_ref().expect("error").code.as_str())
        .collect();
    assert_eq!(codes, ["malformed_tool_call", "duplicate_request_id"]);
    let out = p.run_turn("openai", "m", &turn(vec![good]), "t", 1);
    let e = out.results[0].error.as_ref().expect("error");
    assert_eq!(e.code.as_str(), "duplicate_request_id");
    assert_eq!(e.message, "duplicate request_id: c8");
}

// -- closed-trace retirement (spec/CORE.md, "Closing a trace") ------------------------------------
// Same scenarios and literals in python/tests/test_session_limits.py.

const CYCLES: usize = 100_000;
/// Fresh ids f0..f39 a 32-bit filter holding r0..r7 refuses (sha256 positions, CORE.md).
const RETIRED_FILTER_PARITY: [usize; 8] = [6, 18, 19, 28, 30, 31, 37, 38];

fn open_and_close(p: &mut Pipeline<'_>, trace: &str) {
    assert_eq!(admit(p, trace, "m1", "r1"), ok());
    p.close_trace(trace);
}

fn closed(trace: &str) -> (String, Option<String>, Option<String>) {
    refused("session_closed", &format!("session closed: {trace}"))
}

fn limits(max_closed_traces: usize, retired_filter_bits: usize) -> Limits {
    Limits {
        max_closed_traces,
        retired_filter_bits,
        ..Limits::default()
    }
}

#[test]
fn retirement_defaults() {
    let l = Limits::default();
    assert_eq!(
        (l.max_closed_traces, l.retired_filter_bits),
        (4096, 8_388_608)
    );
    let l: Limits = serde_json::from_str(r#"{"max_closed_traces": 3}"#).unwrap();
    assert_eq!((l.max_closed_traces, l.retired_filter_bits), (3, 8_388_608));
}

#[test]
fn closing_an_unknown_trace_retains_nothing() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, Limits::default());
    for i in 0..10_000 {
        p.close_trace(&format!("never-seen-{i}"));
    }
    assert_eq!(p.closed_trace_state(), (0, 0));
    // The trace was never closed, so its first message is a fresh trace.
    assert_eq!(admit(&mut p, "never-seen-0", "m1", "r1"), ok());
    p.close_trace("never-seen-0");
    assert_eq!(p.closed_trace_state(), (1, 0));
    p.close_trace("never-seen-0"); // closing a closed trace is a no-op too
    assert_eq!(p.closed_trace_state(), (1, 0));
}

#[test]
fn retired_trace_stays_refused() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, limits(2, 8_388_608));
    for t in ["a", "b", "c"] {
        open_and_close(&mut p, t);
    }
    // "a" left the window of the last 2 closed ids and was retired into the filter.
    assert_eq!(p.closed_trace_state(), (2, 1_048_576));
    assert_eq!(admit(&mut p, "a", "m1", "r1"), closed("a")); // the replay
    assert_eq!(admit(&mut p, "a", "m2", "r2"), closed("a")); // and any new message
    assert_eq!(
        p.register_input(input_rec("i1", "a")).unwrap_err(),
        "session closed: a"
    );
    p.close_trace("a"); // closing a retired trace is a no-op
    assert_eq!(p.closed_trace_state(), (2, 1_048_576));
    assert!(!p.ledger.holds("a") && p.inputs("a").is_empty());
    let n = p.events.len();
    assert_eq!(admit(&mut p, "d", "m1", "r1"), ok()); // a fresh trace is admitted
    assert!(p.events.len() > n);
}

#[test]
fn retired_trace_refuses_continuations() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, limits(1, 8_388_608));
    p.run_turn(
        "openai",
        "m",
        &turn(vec![call("c1", "delete_file", json!({"path": "/tmp/x"}))]),
        "t",
        0,
    );
    let pa = p.pending_approval("t", "c1").expect("pending");
    p.close_trace("t");
    open_and_close(&mut p, "u"); // pushes "t" out of the window
    assert_eq!(p.closed_trace_state(), (1, 1_048_576));
    let d = continuation("c1", &pa.approval_id);
    let err = p
        .continue_approval("t", "c1", &d, &pa.request_digest, NOW)
        .unwrap_err();
    assert_eq!(err.as_str(), "session_closed");
    assert_eq!(
        p.cancel_approval("t", "c1").unwrap_err().as_str(),
        "session_closed"
    );
}

#[test]
fn zero_filter_bits_refuses_every_new_trace_after_a_retirement() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, limits(0, 0));
    assert_eq!(admit(&mut p, "held", "m1", "r1"), ok());
    assert_eq!(admit(&mut p, "fresh-before", "m1", "r1"), ok());
    open_and_close(&mut p, "a");
    assert_eq!(p.closed_trace_state(), (0, 0));
    assert_eq!(admit(&mut p, "a", "m1", "r1"), closed("a"));
    assert_eq!(
        admit(&mut p, "never-used", "m1", "r1"),
        closed("never-used")
    );
    assert_eq!(admit(&mut p, "held", "m2", "r2"), ok()); // held traces keep working
}

/// A 32-bit filter makes false refusals frequent; both SDKs refuse exactly the same fresh ids.
#[test]
fn retirement_filter_is_identical_across_sdks() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, limits(0, 32));
    for i in 0..8 {
        open_and_close(&mut p, &format!("r{i}"));
    }
    assert_eq!(p.closed_trace_state(), (0, 4));
    for i in 0..8 {
        let t = format!("r{i}");
        assert_eq!(admit(&mut p, &t, "m2", "r2"), closed(&t));
    }
    let refused: Vec<usize> = (0..40)
        .filter(|i| {
            admit(&mut p, &format!("f{i}"), "m1", "r1").1.as_deref() == Some("session_closed")
        })
        .collect();
    assert_eq!(refused, RETIRED_FILTER_PARITY);
}

#[test]
fn open_close_cycles_stay_within_the_bound_and_replays_stay_refused() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, Limits::default());
    for i in 0..CYCLES {
        open_and_close(&mut p, &format!("s{i}"));
    }
    let l = p.limits;
    let bound = (l.max_closed_traces, l.retired_filter_bits / 8);
    assert_eq!(p.closed_trace_state(), bound);
    assert!(p.events.is_empty() && p.ledger.traces().next().is_none());
    for i in 0..CYCLES {
        let t = format!("s{i}");
        assert_eq!(admit(&mut p, &t, "m1", "r1"), closed(&t));
    }
    assert_eq!(p.closed_trace_state(), bound);
}

/// Before the fix both SDKs recorded the id of an unknown trace on close: its first message was
/// then refused `session_closed`.
#[test]
fn unknown_close_does_not_poison_the_trace() {
    let mut rt = MockRuntime::new();
    let mut p = pipe(&mut rt, Limits::default());
    p.close_trace("never-seen");
    assert_eq!(admit(&mut p, "never-seen", "m1", "r1"), ok());
}
