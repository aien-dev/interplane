//! Every test drives the real interplane `Pipeline` with `AienAuthority`.
use interplane_adapter_aien::*;
use interplane_core::*;
use interplane_crossaxis::MappingTable;
use interplane_crossveil::{Pipeline, RuntimeAuthority, TurnOutcome};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Value};

fn run(rt: &mut AienAuthority, table: MappingTable, name: &str, args: Value) -> TurnOutcome {
    let mut p = Pipeline::new(
        DialectRegistry::with_defaults(),
        table,
        rt,
        Limits::default(),
        RequestLedger::new(),
    );
    let call = json!({"id": "c1", "type": "function", "function": {"name": name, "arguments": args.to_string()}});
    let turn = json!({"role": "assistant", "content": null, "tool_calls": [call]});
    p.run_turn("openai", "m", &turn, "t", 0)
}

fn workspace() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("hello.txt"), "hi from aien").unwrap();
    d
}

#[test]
fn allowed_read_is_ok_with_workspace_provenance() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(true);
    let out = run(&mut rt, table, "read_file", json!({"path": "hello.txt"}));
    let (rec, res) = (&out.records[0], &out.results[0]);
    assert_eq!(res.status, ResultStatus::Ok, "{res:?}");
    assert_eq!(res.data["content"], "hi from aien");
    assert!(rec.decide_invoked && rec.execute_invoked);
    let p = res.provenance.as_ref().unwrap();
    assert_eq!(p.content_kind, Some(ContentKind::WorkspaceContent));
    assert_eq!(p.trust, Some(TrustLevel::WorkspaceUntrusted));
    assert_eq!(p.trusted, Some(false));
    assert_eq!(rt.wire_calls(), 1);
}

#[test]
fn list_dir_ok_and_trusted_flag_is_honoured() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path())
        .unwrap()
        .with_trusted_workspace(true);
    let table = rt.mapping_table(false);
    let out = run(&mut rt, table, "list_dir", json!({"path": "."}));
    let res = &out.results[0];
    assert_eq!(res.status, ResultStatus::Ok, "{res:?}");
    assert_eq!(res.data["entries"], json!(["hello.txt"]));
    let p = res.provenance.as_ref().unwrap();
    assert_eq!(p.trust, Some(TrustLevel::TrustedRuntime));
    assert_eq!(p.trusted, Some(true));
}

#[test]
fn effectful_capability_requires_approval_and_never_executes() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(false);
    let out = run(
        &mut rt,
        table,
        "write_file",
        json!({"path": "x.txt", "content": "y"}),
    );
    let (rec, res) = (&out.records[0], &out.results[0]);
    assert_eq!(res.status, ResultStatus::RequiresApproval);
    assert!(rec.decide_invoked && !rec.execute_invoked);
    let d = res.decision.as_ref().unwrap();
    assert_eq!(d.reason.as_deref(), Some(APPROVAL_REASON));
    assert!(d.approval.is_none());
    let rs = d.runtime_state.as_ref().unwrap();
    assert!(rs.values.contains(&"staged=true".to_string()), "{rs:?}");
    assert_eq!((rt.execute_calls, rt.wire_calls()), (0, 0));
    assert!(!ws.path().join("x.txt").exists());
}

#[test]
fn nonexistent_capability_is_not_found() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = MappingTable::from_value(
        json!({"runtime": "aien", "table_version": "1", "catalog_digest": null,
        "rules": [{"id": "passthrough:teleport", "kind": "passthrough",
                   "from": {"namespace": null, "name": "teleport"}, "to": "teleport"}]}),
    )
    .unwrap();
    let out = run(&mut rt, table, "teleport", json!({"to": "mars"}));
    let rec = &out.records[0];
    assert_eq!(rec.decision.as_deref(), Some("not_found"));
    assert_eq!(rec.error_code.as_deref(), Some("capability_not_found"));
    assert!(!rec.execute_invoked);
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn missing_argument_is_invalid_with_pinned_message() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(false);
    let out = run(&mut rt, table, "read_file", json!({}));
    let (rec, res) = (&out.records[0], &out.results[0]);
    assert_eq!(rec.decision.as_deref(), Some("invalid"));
    assert_eq!(
        res.error.as_ref().unwrap().message,
        "missing required argument: path"
    );
    assert!(!rec.execute_invoked);
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn runtime_failure_is_execution_error() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(false);
    let out = run(&mut rt, table, "read_file", json!({"path": "missing.txt"}));
    let (rec, res) = (&out.records[0], &out.results[0]);
    assert_eq!(res.status, ResultStatus::Error);
    assert_eq!(rec.error_code.as_deref(), Some("execution_error"));
    assert!(rec.execute_invoked);
    assert!(res.error.as_ref().unwrap().message.contains("no such file"));
}

#[test]
fn runtime_absent_fails_closed() {
    let mut rt = AienAuthority::unavailable();
    let table = rt.mapping_table(false);
    let out = run(&mut rt, table, "read_file", json!({"path": "hello.txt"}));
    let (rec, res) = (&out.records[0], &out.results[0]);
    assert_eq!(res.status, ResultStatus::Denied);
    assert_eq!(res.error.as_ref().unwrap().message, UNAVAILABLE);
    assert!(rec.decide_invoked && !rec.execute_invoked);
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn escape_from_workspace_is_denied() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(false);
    for path in ["../etc/passwd", "/etc/passwd"] {
        let out = run(&mut rt, table.clone(), "read_file", json!({"path": path}));
        assert_eq!(out.results[0].status, ResultStatus::Denied, "{path}");
    }
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn aien_gate_denies_unlisted_shell_command() {
    let ws = workspace();
    let mut rt = AienAuthority::new(ws.path()).unwrap();
    let table = rt.mapping_table(false);
    let out = run(&mut rt, table, "bash_eval", json!({"command": "rm -rf /"}));
    assert_eq!(
        out.results[0].status,
        ResultStatus::Denied,
        "{:?}",
        out.results[0]
    );
    assert_eq!(rt.wire_calls(), 0);
}

#[test]
fn catalog_records_both_digests() {
    let ws = workspace();
    let rt = AienAuthority::new(ws.path()).unwrap();
    let c = rt.catalog();
    let a = &c.extensions["aien"];
    assert_eq!(a["catalog_digest"], json!(c.catalog_digest));
    assert_eq!(a["interplane_computed_digest"], json!(c.compute_digest()));
    assert_eq!(a["digests_equal"], json!(false));
    assert_eq!(c.capabilities.len(), 4);
    // A pinned table matches the live catalog; a wrong pin is stale before decide.
    let stale = MappingTable::from_value(json!({"runtime": "aien", "table_version": "1",
        "catalog_digest": format!("sha256:{}", "0".repeat(64)),
        "rules": [{"id": "p", "kind": "passthrough", "from": {"namespace": null, "name": "read_file"}, "to": "read_file"}]}))
    .unwrap();
    let mut rt = rt;
    let out = run(&mut rt, stale, "read_file", json!({"path": "hello.txt"}));
    assert_eq!(
        out.records[0].error_code.as_deref(),
        Some("stale_capability")
    );
    assert_eq!(rt.decide_calls, 0);
}
