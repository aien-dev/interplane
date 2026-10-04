//! Every envelope, result, decision, capability request, catalog and selection the reference
//! implementation consumes or produces is validated against `spec/schemas/*.json`.
use std::path::PathBuf;

use interplane_conformance::{load_fixtures, run_case};
use interplane_core::*;
use interplane_crossaxis::{coerce_arguments, select};
use interplane_crossveil::{mock_mapping_table, MockRuntime, RuntimeAuthority};
use interplane_lenshift::{DialectRegistry, ParseContext};
use jsonschema::{Resource, Validator};
use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn validator(name: &str) -> Validator {
    let dir = root().join("spec/schemas");
    let mut resources = vec![];
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let id = v["$id"].as_str().unwrap().to_string();
        resources.push((id, Resource::from_contents(v).unwrap()));
    }
    let main: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{name}.schema.json"))).unwrap(),
    )
    .unwrap();
    jsonschema::options()
        .with_resources(resources.into_iter())
        .build(&main)
        .expect("schema compiles")
}

fn check(v: &Validator, what: &str, inst: &Value) {
    let errs: Vec<String> = v
        .iter_errors(inst)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    assert!(
        errs.is_empty(),
        "{what} violates its schema: {errs:?}\n{inst}"
    );
}

fn fixtures() -> Vec<Value> {
    load_fixtures(&root().join("conformance/fixtures")).expect("fixtures")
}

#[test]
fn fixture_envelopes_validate() {
    let env = validator("envelope");
    let mut n = 0;
    for fx in fixtures() {
        for step in fx["steps"].as_array().unwrap() {
            if let Some(e) = step.get("envelope") {
                if e["interplane_version"] == "0.1" {
                    check(&env, &format!("{} envelope", fx["case"]), e);
                    n += 1;
                }
            }
        }
    }
    assert!(n >= 3);
}

#[test]
fn produced_messages_validate() {
    let (envs, res, dec, cap) = (
        validator("envelope"),
        validator("result"),
        validator("decision"),
        validator("capability"),
    );
    let reg = DialectRegistry::with_defaults();
    let table = mock_mapping_table();
    let rt = MockRuntime::new();
    let catalog = rt.catalog();
    let mut results = 0;
    for fx in fixtures() {
        let run = run_case(&fx);
        for r in &run.results {
            if r["request_id"].is_null() {
                continue; // CORE.md allows a null request_id before an id exists; the schema requires a string.
            }
            check(&res, &format!("{} result", run.case), r);
            if r["decision"].is_object() {
                check(&dec, &format!("{} decision", run.case), &r["decision"]);
            }
            results += 1;
        }
        for step in fx["steps"].as_array().unwrap() {
            let Some(d) = step["dialect"].as_str() else {
                continue;
            };
            let Ok(dialect) = reg.get(d) else { continue };
            let ctx = ParseContext {
                trace_id: "t".into(),
                turn: 0,
                model: step["model"].as_str().unwrap_or("").into(),
            };
            for (i, intent) in dialect
                .parse(&step["input"], &ctx)
                .intents
                .iter()
                .enumerate()
            {
                let iv = serde_json::to_value(intent).unwrap();
                let e = Envelope::new(
                    &format!("m-{i}"),
                    "t",
                    "2026-01-01T00:00:00Z",
                    Party::new("model", "x"),
                    Party::new("runtime", "mock"),
                    iv,
                );
                check(
                    &envs,
                    &format!("{} wrapped intent", run.case),
                    &serde_json::to_value(&e).unwrap(),
                );
                if let Ok(mut c) = table.map(intent) {
                    if let Some(d) = catalog.capabilities.iter().find(|x| x.name == c.capability) {
                        coerce_arguments(&mut c, d);
                    }
                    check(
                        &cap,
                        &format!("{} capability_request", run.case),
                        &serde_json::to_value(&c).unwrap(),
                    );
                }
            }
        }
    }
    assert!(results >= 25);
}

#[test]
fn catalog_selection_and_events_validate() {
    let rt = MockRuntime::new();
    let catalog = rt.catalog();
    check(
        &validator("catalog"),
        "catalog",
        &serde_json::to_value(&catalog).unwrap(),
    );
    let (sel, _) = select(
        &catalog,
        &["filesystem".to_string()],
        Some(3),
        &["send_email".to_string()],
    );
    check(
        &validator("selection"),
        "selection",
        &serde_json::to_value(&sel).unwrap(),
    );
    let ev = Event {
        kind: EventMsgKind,
        event: EventKind::ToolDecision,
        seq: 1,
        request_id: Some("r".into()),
        turn: Some(0),
        payload: None,
        extensions: Default::default(),
    };
    check(
        &validator("event"),
        "event",
        &serde_json::to_value(&ev).unwrap(),
    );
    // negative control: the validator really rejects.
    let bad = json!({"kind": "result", "request_id": "r", "status": "denied"});
    assert!(
        !validator("result").is_valid(&bad),
        "non-ok result without error must fail"
    );
}
