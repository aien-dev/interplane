use interplane_core::*;
use serde_json::{json, Value};

fn tool_request_payload() -> Value {
    json!({
        "kind": "tool_request", "request_id": "r1",
        "tool": {"namespace": "filesystem", "name": "read"},
        "arguments": {"path": "/a"},
        "provenance": {"dialect": "openai", "parser_version": "1.0.0", "x_prov": 1},
        "x_payload": [1, 2]
    })
}
fn envelope_value() -> Value {
    json!({
        "interplane_version": "0.1", "message_id": "m1", "trace_id": "t1",
        "timestamp": "2026-01-01T00:00:00Z",
        "source": {"kind": "model", "id": "x", "x_party": true},
        "destination": {"kind": "runtime", "id": "mock"},
        "payload": tool_request_payload(),
        "x_top": {"a": 1}
    })
}
fn decision(kind: &str, approval: Option<&str>) -> Decision {
    let mut v = json!({"kind":"decision","request_id":"r1","decision":kind,
        "authority":{"runtime":"mock","policy_engine":"mock.policy"}});
    if let Some(a) = approval {
        v["approval"] = json!({"approval_id": a});
    }
    serde_json::from_value(v).unwrap()
}

// ---- envelope ----
#[test]
fn envelope_positive_roundtrip_and_unknown_fields() {
    let v = envelope_value();
    let e = validate_envelope_value(&v).unwrap();
    assert_eq!(e.extensions["x_top"], json!({"a": 1}));
    assert_eq!(e.source.extensions["x_party"], json!(true));
    let back = serde_json::to_value(&e).unwrap();
    assert_eq!(back, v, "unknown fields survive the round trip");
    let e2: Envelope = serde_json::from_value(back).unwrap();
    assert_eq!(e, e2);
    match e.parse_payload().unwrap() {
        Payload::ToolRequest(t) => {
            assert_eq!(t.extensions["x_payload"], json!([1, 2]));
            assert_eq!(t.provenance.extensions["x_prov"], json!(1));
        }
        _ => panic!("wrong payload"),
    }
}
#[test]
fn envelope_version_major_mismatch() {
    let mut v = envelope_value();
    v["interplane_version"] = json!("2.0");
    assert_eq!(
        validate_envelope_value(&v).unwrap_err(),
        ErrorCode::UnsupportedVersion
    );
    // checked first, even when the rest is junk
    assert_eq!(
        validate_envelope_value(&json!({"interplane_version": "1.0"})).unwrap_err(),
        ErrorCode::UnsupportedVersion
    );
    v["interplane_version"] = json!("0.7");
    assert!(
        validate_envelope_value(&v).is_ok(),
        "minor differences are fine"
    );
}
#[test]
fn envelope_negative_cases() {
    let cases: Vec<(&str, Value, ErrorCode)> = vec![
        (
            "bad message id",
            {
                let mut v = envelope_value();
                v["message_id"] = json!("has space");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "too long id",
            {
                let mut v = envelope_value();
                v["trace_id"] = json!("a".repeat(129));
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "bad timestamp",
            {
                let mut v = envelope_value();
                v["timestamp"] = json!("yesterday");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "bad party kind",
            {
                let mut v = envelope_value();
                v["source"]["kind"] = json!("alien");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "unknown payload kind",
            {
                let mut v = envelope_value();
                v["payload"]["kind"] = json!("telepathy");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "arguments not object",
            {
                let mut v = envelope_value();
                v["payload"]["arguments"] = json!([1]);
                v
            },
            ErrorCode::MalformedToolCall,
        ),
        (
            "missing tool",
            {
                let mut v = envelope_value();
                v["payload"].as_object_mut().unwrap().remove("tool");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
        (
            "bad request id",
            {
                let mut v = envelope_value();
                v["payload"]["request_id"] = json!("a b");
                v
            },
            ErrorCode::MalformedEnvelope,
        ),
    ];
    for (name, v, code) in cases {
        assert_eq!(validate_envelope_value(&v).unwrap_err(), code, "{name}");
    }
}
#[test]
fn envelope_malformed_input() {
    for v in [
        json!(null),
        json!([]),
        json!("x"),
        json!({}),
        json!({"interplane_version": 1}),
    ] {
        assert_eq!(
            validate_envelope_value(&v).unwrap_err(),
            ErrorCode::MalformedEnvelope
        );
    }
    let mut v = envelope_value();
    v.as_object_mut().unwrap().remove("payload");
    assert_eq!(
        validate_envelope_value(&v).unwrap_err(),
        ErrorCode::MalformedEnvelope
    );
}

// ---- tool_request ----
#[test]
fn tool_request_roundtrip_const_kind() {
    let t: ToolRequest = serde_json::from_value(tool_request_payload()).unwrap();
    let again: ToolRequest = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert_eq!(t, again);
    let mut bad = tool_request_payload();
    bad["kind"] = json!("decision");
    assert!(serde_json::from_value::<ToolRequest>(bad).is_err());
    let mut noargs = tool_request_payload();
    noargs.as_object_mut().unwrap().remove("arguments");
    assert!(serde_json::from_value::<ToolRequest>(noargs).is_err());
}

// ---- error codes ----
#[test]
fn error_code_unknown_is_preserved() {
    let c: ErrorCode = serde_json::from_value(json!("from_the_future")).unwrap();
    assert_eq!(c, ErrorCode::Unknown("from_the_future".into()));
    assert_eq!(serde_json::to_value(&c).unwrap(), json!("from_the_future"));
    assert_eq!(
        serde_json::to_value(ErrorCode::PolicyDenied).unwrap(),
        json!("policy_denied")
    );
    assert!(serde_json::from_value::<ErrorCode>(json!(5)).is_err());
}

// ---- decision ----
#[test]
fn decision_roundtrip_and_unknown_value_is_never_authorized() {
    let mut v = json!({"kind":"decision","request_id":"r1","decision":"authorized",
        "authority":{"runtime":"mock","policy_engine":"p","x":1},"constraints":["c"],"x_top":true});
    let d: Decision = serde_json::from_value(v.clone()).unwrap();
    assert!(d.is_authorized());
    let ser = serde_json::to_value(&d).unwrap();
    assert_eq!(ser["x_top"], json!(true));
    assert_eq!(ser["authority"]["x"], json!(1));
    assert_eq!(
        ser["reason"],
        Value::Null,
        "pinned shape: nulls are emitted"
    );
    assert_eq!(serde_json::from_value::<Decision>(ser).unwrap(), d);
    v["decision"] = json!("super_authorized");
    let d: Decision = serde_json::from_value(v).unwrap();
    assert_eq!(d.decision, DecisionKind::Unknown("super_authorized".into()));
    assert!(!d.is_authorized());
    assert!(serde_json::from_value::<Decision>(
        json!({"kind":"decision","request_id":"r","decision":"authorized"})
    )
    .is_err());
}

// ---- result ----
#[test]
fn result_roundtrip_digest_ignores_duration() {
    let mut r = ToolResult::ok("r1", json!({"a": 1}));
    r.provenance.as_mut().unwrap().duration_ms = Some(5);
    let d1 = r.result_digest();
    r.provenance.as_mut().unwrap().duration_ms = Some(900);
    assert_eq!(d1, r.result_digest());
    let back: ToolResult = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(back, r);
    assert!(d1.starts_with("sha256:") && d1.len() == 71);
    let f = ToolResult::failed(
        None,
        ResultStatus::Rejected,
        ErrorCode::MalformedEnvelope,
        "x",
    );
    assert_eq!(f.request_id, None);
    assert_eq!(serde_json::to_value(&f).unwrap()["request_id"], Value::Null);
}

// ---- other payloads: round trip + unknown field ----
#[test]
fn other_payloads_roundtrip() {
    let docs = vec![
        json!({"kind":"capability_request","request_id":"r","runtime":"mock","capability":"read_file","arguments":{},
               "tool":{"namespace":null,"name":"read_file"},"mapping":{"table_version":"1","rule_id":"passthrough:read_file","passthrough":true,"catalog_digest":"sha256:".to_string()+&"0".repeat(64),"coerced":["n"],"y":2},"z":1}),
        json!({"kind":"event","event":"decided","seq":3,"request_id":"r","turn":0,"payload":{"a":1},"z":1}),
        json!({"kind":"catalog","runtime":"mock","catalog_version":"1","capabilities":[
            {"name":"read_file","description":"d","parameters":{"type":"object"},"domains":["filesystem"],"z":1}],"z":1}),
        json!({"kind":"selection","runtime":"mock","catalog_digest":"sha256:".to_string()+&"0".repeat(64),
               "selector":{"name":"domain_match","version":"1","max_capabilities":3},"requested_domains":["a"],
               "selected":[{"name":"x","rule_id":"domain_match"}],"excluded":[{"name":"y","reason":"domain_mismatch"}],
               "measure":{"full_count":2,"selected_count":1,"q":1},"z":1}),
        json!({"kind":"probe_report","probe_version":"0.1.0","endpoint":"http://h","model":"m","started_at":"2026-01-01T00:00:00Z",
               "probes":[{"name":"chat.basic","verdict":"PASS","z":1}],
               "profiles":[{"name":"interplane.core.0.1","status":"untested","required_probes":["chat.basic"]}],"z":1}),
    ];
    for d in docs {
        let p = Payload::from_value(&d).unwrap();
        let ser = match &p {
            Payload::CapabilityRequest(x) => serde_json::to_value(x),
            Payload::Event(x) => serde_json::to_value(x),
            Payload::Catalog(x) => serde_json::to_value(x),
            Payload::Selection(x) => serde_json::to_value(x),
            Payload::ProbeReport(x) => serde_json::to_value(x),
            _ => unreachable!(),
        }
        .unwrap();
        assert_eq!(ser, d);
        assert_eq!(Payload::from_value(&ser).unwrap(), p);
    }
    assert_eq!(
        Payload::from_value(&json!({"kind": "nope"})).unwrap_err(),
        ErrorCode::MalformedEnvelope
    );
    assert_eq!(
        Payload::from_value(&json!({"kind": "event"})).unwrap_err(),
        ErrorCode::MalformedEnvelope
    );
}

#[test]
fn catalog_digest_is_order_independent() {
    let a = json!({"kind":"catalog","runtime":"m","catalog_version":"1","capabilities":[
        {"name":"a","description":"","parameters":{}},{"name":"b","description":"","parameters":{"type":"object"}}]});
    let mut b = a.clone();
    b["capabilities"].as_array_mut().unwrap().reverse();
    let ca: Catalog = serde_json::from_value(a).unwrap();
    let cb: Catalog = serde_json::from_value(b).unwrap();
    assert_eq!(ca.compute_digest(), cb.compute_digest());
}

// ---- lifecycle ----
#[test]
fn lifecycle_happy_path() {
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("authorized", None)).unwrap(),
        State::Authorized
    );
    assert_eq!(
        l.begin_execution(&decision("authorized", None)).unwrap(),
        State::Executing
    );
    assert_eq!(l.finish(State::Succeeded).unwrap(), State::Succeeded);
    assert!(l.state().is_terminal());
    assert!(l.finish(State::Failed).is_err());
}
#[test]
fn lifecycle_denied_is_terminal_and_unknown_is_denied() {
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("denied", None)).unwrap(),
        State::Denied
    );
    assert!(l.apply_decision(&decision("authorized", None)).is_err());
    assert!(l
        .apply_decision(&decision("authorized", Some("a")))
        .is_err());
    assert!(l.begin_execution(&decision("authorized", None)).is_err());
    assert_eq!(l.state(), State::Denied);
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("weird", None)).unwrap(),
        State::Denied
    );
}
#[test]
fn lifecycle_cannot_skip_the_decision() {
    let mut l = Lifecycle::new("r1");
    assert!(l.begin_execution(&decision("authorized", None)).is_err());
    assert!(
        l.apply_decision(&decision("authorized", None)).is_err(),
        "not mapped yet"
    );
    l.map().unwrap();
    assert!(
        l.begin_execution(&decision("authorized", None)).is_err(),
        "mapped is not authorized"
    );
    assert!(l.finish(State::Succeeded).is_err());
    // a non-authorized decision can never drive execution
    l.apply_decision(&decision("authorized", None)).unwrap();
    assert_eq!(
        l.begin_execution(&decision("denied", None)),
        Err(LifecycleError::NotAuthorized)
    );
    assert_eq!(
        l.begin_execution(&decision("weird", None)),
        Err(LifecycleError::NotAuthorized)
    );
}
#[test]
fn lifecycle_approval_needs_new_decision_with_approval_id() {
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("requires_approval", Some("ap1")))
            .unwrap(),
        State::RequiresApproval
    );
    assert_eq!(
        l.apply_decision(&decision("authorized", None)),
        Err(LifecycleError::MissingApproval)
    );
    assert_eq!(l.state(), State::RequiresApproval);
    assert_eq!(
        l.apply_decision(&decision("authorized", Some("ap1")))
            .unwrap(),
        State::Authorized
    );
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    l.apply_decision(&decision("requires_approval", Some("ap1")))
        .unwrap();
    assert_eq!(
        l.apply_decision(&decision("denied", Some("ap1"))).unwrap(),
        State::Denied
    );
}
#[test]
fn lifecycle_continuation_must_cite_the_minted_approval_id() {
    // Mirrors python/tests/test_core.py test_requires_approval_needs_new_decision_citing_id.
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    l.apply_decision(&decision("requires_approval", Some("A1")))
        .unwrap();
    for bad in [Some("forged"), Some(""), Some("mock-approval-r2"), None] {
        assert_eq!(
            l.apply_decision(&decision("authorized", bad)),
            Err(LifecycleError::MissingApproval),
            "{bad:?} must be refused"
        );
        assert_eq!(l.state(), State::RequiresApproval);
    }
    assert_eq!(
        l.apply_decision(&decision("authorized", Some("A1")))
            .unwrap(),
        State::Authorized
    );
    // A request whose requires_approval decision minted no id can never be continued.
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    l.apply_decision(&decision("requires_approval", None))
        .unwrap();
    assert_eq!(
        l.apply_decision(&decision("authorized", Some("A1"))),
        Err(LifecycleError::MissingApproval)
    );
    // With the minted id, every value but authorized is DENIED.
    for kind in ["requires_approval", "not_found", "invalid", "weird"] {
        let mut l = Lifecycle::new("r1");
        l.map().unwrap();
        l.apply_decision(&decision("requires_approval", Some("A1")))
            .unwrap();
        assert_eq!(
            l.apply_decision(&decision(kind, Some("A1"))).unwrap(),
            State::Denied,
            "{kind}"
        );
    }
}
#[test]
fn lifecycle_rejections_and_wrong_request() {
    let mut l = Lifecycle::new("r1");
    assert_eq!(l.reject().unwrap(), State::Rejected);
    assert!(l.map().is_err());
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("not_found", None)).unwrap(),
        State::Rejected
    );
    let mut l = Lifecycle::new("r1");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("invalid", None)).unwrap(),
        State::Rejected
    );
    let mut l = Lifecycle::new("other");
    l.map().unwrap();
    assert_eq!(
        l.apply_decision(&decision("authorized", None)),
        Err(LifecycleError::WrongRequest)
    );
}

// ---- property tests ----
mod props {
    use super::*;
    use proptest::prelude::*;

    fn arb_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|i| json!(i)),
            "[ -~é\u{1f600}\n\t]{0,12}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::hash_map("[a-z]{0,4}", inner, 0..4)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }
    proptest! {
        #[test]
        fn canonical_parses_back_equal_and_is_stable(v in arb_json()) {
            let c = canonicalize(&v);
            let back: Value = serde_json::from_str(&c).unwrap();
            prop_assert_eq!(&back, &v);
            prop_assert_eq!(canonicalize(&back), c);
        }
        #[test]
        fn id_pattern_matches_reference(s in "[ -~]{0,10}") {
            let expect = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c));
            prop_assert_eq!(is_valid_id(&s), expect);
        }
    }
}

#[test]
fn result_shape_is_pinned() {
    let r = ToolResult::failed(
        Some("r1"),
        ResultStatus::Denied,
        ErrorCode::RuntimeUnavailable,
        "m",
    );
    let v = serde_json::to_value(&r).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "data",
            "decision",
            "error",
            "kind",
            "provenance",
            "request_id",
            "status"
        ]
    );
    let pk: Vec<&String> = v["provenance"].as_object().unwrap().keys().collect();
    assert_eq!(
        pk,
        [
            "capability",
            "content_kind",
            "duration_ms",
            "runtime",
            "trust",
            "trusted"
        ]
    );
    assert_eq!(
        v["error"],
        json!({"code": "runtime_unavailable", "message": "m", "retryable": true})
    );
    let t = ToolResult::failed(
        Some("r1"),
        ResultStatus::Error,
        ErrorCode::ExecutionError,
        "m",
    );
    assert_eq!(
        serde_json::to_value(&t).unwrap()["error"]["retryable"],
        json!(false)
    );
}

#[test]
fn trust_levels_and_unknown_values() {
    assert_eq!(
        TrustLevel::parse("trusted_runtime").trusted_flag(),
        Some(true)
    );
    assert_eq!(
        TrustLevel::parse("external_untrusted").trusted_flag(),
        Some(false)
    );
    assert_eq!(TrustLevel::parse("unknown").trusted_flag(), None);
    assert_eq!(TrustLevel::parse("user_supplied").trusted_flag(), None);
    assert_eq!(
        TrustLevel::parse("godmode").trusted_flag(),
        Some(false),
        "never trusted by accident"
    );
    assert_eq!(ContentKind::parse("web_content").as_str(), "web_content");
}

#[test]
fn input_record_examples_roundtrip_and_null_parent_is_explicit() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/input");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(e.unwrap().path()).unwrap()).unwrap();
        let rec: InputRecord = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&rec).unwrap(), v);
        n += 1;
    }
    assert_eq!(n, 9);
    let mut v = json!({
        "input_id": "i", "content_kind": "memory", "trust": "made_up", "source": {"kind": "runtime", "id": "r"},
        "origin": "m", "content_digest": "sha256:00", "trace_id": "t", "x": 1
    });
    let rec: InputRecord = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(rec.parent_id, None);
    assert!(rec.derived_from.is_empty());
    v["parent_id"] = Value::Null;
    v["derived_from"] = json!([]);
    assert_eq!(serde_json::to_value(&rec).unwrap(), v);
}

#[test]
fn exposure_roundtrips_inside_provenance_extensions() {
    let mut p = tool_request_payload();
    p["provenance"]["exposure"] = json!({"inputs": ["a"], "floor": "unknown"});
    let tr: ToolRequest = serde_json::from_value(p.clone()).unwrap();
    assert_eq!(serde_json::to_value(&tr).unwrap(), p);
    let ex: Exposure = serde_json::from_value(p["provenance"]["exposure"].clone()).unwrap();
    assert_eq!(ex.floor, TrustLevel::Undetermined);
}

#[test]
fn cancel_only_leaves_requires_approval_and_only_to_denied() {
    let mut l = Lifecycle::new("r");
    assert!(l.cancel().is_err());
    l.map().unwrap();
    assert!(l.cancel().is_err());
    let d: Decision = serde_json::from_value(serde_json::json!({
        "kind": "decision", "request_id": "r", "decision": "requires_approval",
        "authority": {"runtime": "m", "policy_engine": "p"},
        "approval": {"approval_id": "a1"}
    }))
    .unwrap();
    l.apply_decision(&d).unwrap();
    assert_eq!(l.cancel(), Ok(State::Denied));
    assert!(l.cancel().is_err());
    assert_eq!(l.state(), State::Denied);
}
