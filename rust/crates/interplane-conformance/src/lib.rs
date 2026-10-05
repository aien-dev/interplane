//! Conformance runner: executes `conformance/fixtures/NN-*.json` through a fresh pipeline and
//! mock runtime per case and compares the result with `expected`.
use std::path::Path;

use interplane_core::{
    canonicalize, digest, Decision, Lifecycle, Limits, RequestLedger, ToolResult,
};
use interplane_crossaxis::{expand, select};
use interplane_crossveil::{
    mock_mapping_table, MockRuntime, ObservedRecord, Pipeline, RuntimeAuthority,
};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Map, Value};

/// What running one case produced.
#[derive(Debug, Clone)]
pub struct CaseRun {
    pub case: String,
    pub observed: Vec<ObservedRecord>,
    pub turns: Vec<Value>,
    pub decide_calls: u32,
    pub execute_calls: u32,
    /// Every produced `result` payload, keyed by request id (last one wins), in order.
    pub results: Vec<Value>,
    /// The selection receipt after each `expand` step, in order.
    pub selections: Vec<Value>,
    /// The trace's input ledger after the last step (0.3 cut P3), in registration order.
    pub inputs: Vec<Value>,
    /// `{request_id, inputs, floor}` the runtime saw in `CallContext` at `decide`, in order.
    pub exposure: Vec<Value>,
}

/// Run one fixture document.
pub fn run_case(fx: &Value) -> CaseRun {
    let case = fx["case"].as_str().unwrap_or("unnamed").to_string();
    let trace = fx["trace_id"].as_str().unwrap_or("trace").to_string();
    let limits: Limits = serde_json::from_value(fx["limits"].clone()).unwrap_or_default();
    let mut rt = MockRuntime::new();
    if let Some(o) = fx["mock_provenance"].as_object() {
        rt.provenance_overrides = o.clone();
    }
    let mut observed = vec![];
    let mut turns = vec![];
    let mut results = vec![];
    let mut selections = vec![];
    let mut inputs: Vec<Value> = vec![];
    let catalog = rt.catalog();
    let mut sel = fx.get("selection").map(|spec| {
        let strings = |k: &str| -> Vec<String> {
            spec[k]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        select(
            &catalog,
            &strings("requested_domains"),
            spec["max_capabilities"].as_u64().map(|m| m as usize),
            &strings("always_include"),
        )
        .0
    });
    {
        let mut p = Pipeline::new(
            DialectRegistry::with_defaults(),
            mock_mapping_table(),
            &mut rt,
            limits,
            RequestLedger::new(),
        );
        p.message_prefix = format!("m-{case}");
        if let Some(name) = fx["mapping_table"].as_str() {
            if let Some(t) = interplane_crossveil::mock_table_by_name(name) {
                p.table = t;
            }
        }
        for step in fx["steps"].as_array().cloned().unwrap_or_default() {
            let turn = step["turn"].as_u64().unwrap_or(0);
            if let Some(ex) = step.get("expand") {
                if let Some(cur) = sel.take() {
                    let next = expand(
                        &cur,
                        &catalog,
                        &ex["evidence"],
                        ex["max_expansions"].as_u64(),
                        ex["max_added_per_expansion"].as_u64(),
                    );
                    match next {
                        Ok(n) => {
                            selections.push(serde_json::to_value(&n).unwrap_or(Value::Null));
                            sel = Some(n);
                        }
                        Err(e) => selections.push(json!({"error": e.to_string()})),
                    }
                }
            } else if let Some(env) = step.get("envelope") {
                let (r, o) = p.admit_value(env);
                results.push(serde_json::to_value(&r).unwrap_or(Value::Null));
                observed.push(o);
            } else if step.get("input").is_some() && step.get("dialect").is_none() {
                // Host-only input registration (0.3 cut P3): never reachable from model input.
                if let Ok(rec) = serde_json::from_value(step["input"].clone()) {
                    let _ = p.register_input(rec);
                }
            } else if let Some(d) = step["dialect"].as_str() {
                let model = step["model"].as_str().unwrap_or("");
                let out = p.run_turn(d, model, &step["input"], &trace, turn);
                let mut t = json!({"turn": turn, "outcome": out.outcome, "intents": out.intents.len(),
                    "rejected": out.rejected.len(), "text": out.text});
                if let Some(c) = &out.error_code {
                    t["error_code"] = json!(c.as_str());
                }
                turns.push(t);
                results.extend(
                    out.results
                        .iter()
                        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null)),
                );
                observed.extend(out.records);
            }
        }
        // Reported only for cases that assert them, so older verdicts keep their shape.
        if fx["expected"].get("inputs").is_some() {
            inputs = p
                .inputs(&trace)
                .iter()
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .collect();
        }
    }
    CaseRun {
        case,
        observed,
        turns,
        decide_calls: rt.decide_calls,
        execute_calls: rt.execute_calls,
        results,
        selections,
        inputs,
        exposure: if fx["expected"].get("exposure").is_some() {
            rt.seen_exposure.clone()
        } else {
            vec![]
        },
    }
}

fn dotted<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match seg.parse::<usize>() {
            Ok(i) if cur.is_array() => cur.get(i)?,
            _ => cur.get(seg)?,
        };
    }
    Some(cur)
}

/// Compare a run with `expected`; returns the list of disagreements (empty = pass).
pub fn compare(fx: &Value, run: &CaseRun) -> Vec<String> {
    let mut errs = vec![];
    let exp = &fx["expected"];
    let want: Vec<Value> = exp["observed"].as_array().cloned().unwrap_or_default();
    if want.len() != run.observed.len() {
        errs.push(format!(
            "observed: expected {} records, got {}",
            want.len(),
            run.observed.len()
        ));
    }
    for (i, (w, g)) in want.iter().zip(&run.observed).enumerate() {
        let mut got = serde_json::to_value(g).unwrap_or(Value::Null);
        if w.get("result_digest").is_none_or(Value::is_null) {
            got["result_digest"] = Value::Null;
        }
        let mut w = w.clone();
        if w.get("result_digest").is_none() {
            w["result_digest"] = Value::Null;
        }
        if got != w {
            errs.push(format!(
                "observed[{i}]: expected {}, got {}",
                canonicalize(&w),
                canonicalize(&got)
            ));
        }
    }
    let want_turns = exp["turns"].as_array().cloned().unwrap_or_default();
    if want_turns != run.turns {
        errs.push(format!(
            "turns: expected {}, got {}",
            canonicalize(&Value::Array(want_turns)),
            canonicalize(&Value::Array(run.turns.clone()))
        ));
    }
    let rt = &exp["runtime"];
    if rt["decide_calls"].as_u64() != Some(run.decide_calls as u64)
        || rt["execute_calls"].as_u64() != Some(run.execute_calls as u64)
    {
        errs.push(format!(
            "runtime: expected {rt}, got decide={} execute={}",
            run.decide_calls, run.execute_calls
        ));
    }
    if let Some(want) = exp.get("selections") {
        if want != &Value::Array(run.selections.clone()) {
            errs.push("selections differ from expected".to_string());
        }
    }
    if let Some(want) = exp.get("inputs").and_then(Value::as_array) {
        // Subset match per record: a fixture may omit `content_digest` of an automatic record.
        let ok = want.len() == run.inputs.len()
            && want.iter().zip(&run.inputs).all(|(w, g)| {
                w.as_object()
                    .is_some_and(|o| o.iter().all(|(k, v)| g.get(k) == Some(v)))
            });
        if !ok {
            errs.push(format!(
                "inputs: expected {}, got {}",
                canonicalize(&Value::Array(want.clone())),
                canonicalize(&Value::Array(run.inputs.clone()))
            ));
        }
    }
    if let Some(want) = exp.get("exposure") {
        if want != &Value::Array(run.exposure.clone()) {
            errs.push(format!(
                "exposure: expected {}, got {}",
                canonicalize(want),
                canonicalize(&Value::Array(run.exposure.clone()))
            ));
        }
    }
    for c in exp["result_checks"].as_array().cloned().unwrap_or_default() {
        let rid = c["request_id"].as_str().unwrap_or("");
        let path = c["path"].as_str().unwrap_or("");
        let result = run
            .results
            .iter()
            .find(|r| r["request_id"].as_str() == Some(rid));
        // `round_trip`: read the value after JCS serialization and a parse back into the
        // SDK's own result type, as a receiver would see it.
        let parsed = if c["round_trip"] == json!(true) {
            result
                .and_then(|r| serde_json::from_str::<ToolResult>(&canonicalize(r)).ok())
                .and_then(|t| serde_json::to_value(t).ok())
        } else {
            result.cloned()
        };
        let found = parsed.as_ref().and_then(|r| dotted(r, path));
        if found != Some(&c["equals"]) {
            errs.push(format!(
                "result_check {rid}:{path}: expected {}, got {:?}",
                c["equals"], found
            ));
        }
    }
    errs
}

/// One row of the verdict file.
pub fn verdict(run: &CaseRun, errs: &[String]) -> Value {
    let mut v = json!({
        "pass": errs.is_empty(),
        "observed": serde_json::to_value(&run.observed).unwrap_or(Value::Null),
        "turns": run.turns,
        "runtime": {"decide_calls": run.decide_calls, "execute_calls": run.execute_calls},
    });
    if !run.selections.is_empty() {
        v["selections"] = Value::Array(run.selections.clone());
    }
    if !run.inputs.is_empty() {
        v["inputs"] = Value::Array(run.inputs.clone());
    }
    if !run.exposure.is_empty() {
        v["exposure"] = Value::Array(run.exposure.clone());
    }
    v
}

/// Load `NN-*.json` fixtures (not subdirectories), sorted by file name.
pub fn load_fixtures(dir: &Path) -> Result<Vec<Value>, String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            p.is_file()
                && n.ends_with(".json")
                && n.len() > 3
                && n.as_bytes()[..2].iter().all(u8::is_ascii_digit)
                && n.as_bytes()[2] == b'-'
        })
        .collect();
    names.sort();
    names
        .iter()
        .map(|p| {
            let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

/// Check every `digest/*.json` fixture. A fixture with a `"type"` is also parsed as that SDK type
/// and re-serialized before the JCS and digest are compared. Returns `(name, error)` per fixture.
pub fn check_digest_fixtures(dir: &Path) -> Vec<(String, Option<String>)> {
    let mut paths: Vec<_> = match std::fs::read_dir(dir.join("digest")) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let name = format!("digest/{}", p.file_stem().unwrap().to_string_lossy());
            (name, check_digest_file(p))
        })
        .collect()
}

fn check_digest_file(p: &Path) -> Option<String> {
    let fx: Value = match std::fs::read_to_string(p)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => return Some(e),
    };
    let value = match fx.get("type").and_then(Value::as_str) {
        None => fx["value"].clone(),
        Some(t) => match typed_roundtrip(t, &fx["value"]) {
            Ok(v) => v,
            Err(e) => return Some(e),
        },
    };
    let got = canonicalize(&value);
    if fx["expected_jcs"].as_str() != Some(got.as_str()) {
        return Some(format!("jcs mismatch: got {got}"));
    }
    if fx["expected_sha256"].as_str() != Some(digest(&value).as_str()) {
        return Some("sha256 mismatch".into());
    }
    None
}

/// Parse `value` as the named SDK type and serialize it back.
fn typed_roundtrip(ty: &str, value: &Value) -> Result<Value, String> {
    let e = |e: serde_json::Error| e.to_string();
    match ty {
        "InputRecord" => {
            let t: interplane_core::InputRecord =
                serde_json::from_value(value.clone()).map_err(e)?;
            serde_json::to_value(t).map_err(e)
        }
        "Exposure" => {
            let t: interplane_core::Exposure = serde_json::from_value(value.clone()).map_err(e)?;
            serde_json::to_value(t).map_err(e)
        }
        _ => Err(format!("unknown fixture type {ty}")),
    }
}

/// Silence an unused-import lint for `RuntimeAuthority` in docs builds.
#[doc(hidden)]
pub fn _runtime_id(r: &dyn RuntimeAuthority) -> String {
    r.runtime_id().to_string()
}

/// Build the verdicts object `{case: {...}}`.
pub fn verdicts(rows: Vec<(String, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in rows {
        m.insert(k, v);
    }
    Value::Object(m)
}

/// Load `lifecycle/NN-*.json` fixtures, sorted by file name. Absent directory = none.
pub fn load_lifecycle_fixtures(dir: &Path) -> Result<Vec<Value>, String> {
    let sub = dir.join("lifecycle");
    if !sub.is_dir() {
        return Ok(vec![]);
    }
    load_fixtures(&sub)
}

/// Drive the Crossveil lifecycle directly (no pipeline): for each step, the lifecycle of
/// `request_id` (created and mapped on first sight) receives `decision`. Each step yields the
/// state after the call and whether the call was refused. Returns `(rows, disagreements)`.
pub fn run_lifecycle_case(fx: &Value) -> (Vec<Value>, Vec<String>) {
    let mut lcs: Vec<Lifecycle> = vec![];
    let mut rows = vec![];
    for step in fx["steps"].as_array().cloned().unwrap_or_default() {
        let rid = step["request_id"].as_str().unwrap_or("").to_string();
        let idx = match lcs.iter().position(|l| l.request_id() == rid) {
            Some(i) => i,
            None => {
                let mut l = Lifecycle::new(&rid);
                let _ = l.map();
                lcs.push(l);
                lcs.len() - 1
            }
        };
        let lc = &mut lcs[idx];
        let refused = match serde_json::from_value::<Decision>(step["decision"].clone()) {
            Ok(d) => lc.apply_decision(&d).is_err(),
            Err(_) => true,
        };
        rows.push(json!({"request_id": rid, "state": lc.state().name(), "refused": refused}));
    }
    let want = fx["expected"].as_array().cloned().unwrap_or_default();
    let mut errs = vec![];
    if want != rows {
        errs.push(format!(
            "steps: expected {}, got {}",
            canonicalize(&Value::Array(want)),
            canonicalize(&Value::Array(rows.clone()))
        ));
    }
    (rows, errs)
}

/// One row of the verdict file for a lifecycle case.
pub fn lifecycle_verdict(rows: &[Value], errs: &[String]) -> Value {
    json!({"pass": errs.is_empty(), "steps": rows})
}
