//! Conformance runner: executes `conformance/fixtures/NN-*.json` through a fresh pipeline and
//! mock runtime per case and compares the result with `expected`.
use std::path::Path;

use interplane_core::{canonicalize, digest, Limits, RequestLedger};
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
            if let Some(env) = step.get("envelope") {
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
    json!({
        "pass": errs.is_empty(),
        "observed": serde_json::to_value(&run.observed).unwrap_or(Value::Null),
        "turns": run.turns,
        "runtime": {"decide_calls": run.decide_calls, "execute_calls": run.execute_calls},
    })
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
