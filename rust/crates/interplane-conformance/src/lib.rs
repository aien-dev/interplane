//! Conformance runner: executes `conformance/fixtures/NN-*.json` through a fresh pipeline and
//! mock runtime per case and compares the result with `expected`.
use std::path::Path;

use interplane_core::{canonicalize, digest, Decision, Lifecycle, Limits, RequestLedger};
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
}

/// Run one fixture document.
pub fn run_case(fx: &Value) -> CaseRun {
    let case = fx["case"].as_str().unwrap_or("unnamed").to_string();
    let trace = fx["trace_id"].as_str().unwrap_or("trace").to_string();
    let limits: Limits = serde_json::from_value(fx["limits"].clone()).unwrap_or_default();
    let mut rt = MockRuntime::new();
    let mut observed = vec![];
    let mut turns = vec![];
    let mut results = vec![];
    let mut selections = vec![];
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
    }
    CaseRun {
        case,
        observed,
        turns,
        decide_calls: rt.decide_calls,
        execute_calls: rt.execute_calls,
        results,
        selections,
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
    for c in exp["result_checks"].as_array().cloned().unwrap_or_default() {
        let rid = c["request_id"].as_str().unwrap_or("");
        let path = c["path"].as_str().unwrap_or("");
        let found = run
            .results
            .iter()
            .find(|r| r["request_id"].as_str() == Some(rid))
            .and_then(|r| dotted(r, path));
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

/// Check the JCS digest fixture (`digest/jcs-01.json`) if present. Returns an error text on mismatch.
pub fn check_digest_fixture(dir: &Path) -> Option<String> {
    let p = dir.join("digest").join("jcs-01.json");
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let got = canonicalize(&fx["value"]);
    if fx["expected_jcs"].as_str() != Some(got.as_str()) {
        return Some(format!("jcs mismatch: got {got}"));
    }
    if fx["expected_sha256"].as_str() != Some(digest(&fx["value"]).as_str()) {
        return Some("sha256 mismatch".into());
    }
    None
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
