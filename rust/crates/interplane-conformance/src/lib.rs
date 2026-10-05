//! Conformance runner: executes `conformance/fixtures/NN-*.json` through a fresh pipeline and
//! mock runtime per case and compares the result with `expected`.
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use interplane_core::{
    canonicalize, digest, CapabilityRequest, Catalog, Decision, Lifecycle, Limits, RequestLedger,
    ToolResult,
};
use interplane_crossaxis::{expand, select};
use interplane_crossveil::pipeline::{capability_request_digest, PendingApproval};
use interplane_crossveil::{
    mock_mapping_table, CallContext, MockRuntime, ObservedRecord, Pipeline, RuntimeAuthority,
};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Map, Value};

#[cfg(feature = "negative-controls")]
pub mod negctl;

/// Which negative control (if any) a run applies. Without the `negative-controls` feature there
/// is none, and nothing below can name one.
#[cfg(feature = "negative-controls")]
pub type Ctl = Option<negctl::Variant>;
#[cfg(not(feature = "negative-controls"))]
pub type Ctl = Option<std::convert::Infallible>;

/// Capabilities the mock classes as effects (CORE.md has no effect-class column until cut E1; this
/// is the harness-local set). `read_*`, `list_dir`, `web_fetch` and the like may run.
pub const EFFECT_CAPABILITIES: [&str; 4] =
    ["append_note", "write_file", "delete_file", "send_email"];

/// Wraps the mock runtime to record every request it decides and every id it executes, so the
/// injection judge can say what ran. Behaves exactly like the mock unless a negative control is
/// built in and selected.
struct Probe<'a> {
    inner: &'a mut MockRuntime,
    /// `(request_id, capability, canonical arguments)` per `decide`, in order.
    requests: Vec<(String, String, String)>,
    executed: HashSet<String>,
    /// Harness-only (fixture `mock` step): the catalog digest the runtime reports instead of its own.
    catalog_override: Rc<RefCell<Option<String>>>,
    /// Harness-only: true while a negative control re-admits an original request in place of a
    /// continuation.
    #[cfg_attr(not(feature = "negative-controls"), allow(dead_code))]
    readmit: Rc<Cell<bool>>,
    #[cfg(feature = "negative-controls")]
    ctl: Ctl,
}

impl RuntimeAuthority for Probe<'_> {
    fn runtime_id(&self) -> &str {
        self.inner.runtime_id()
    }
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        self.requests.push((
            req.request_id.clone(),
            req.capability.clone(),
            canonicalize(&Value::Object(req.arguments.clone())),
        ));
        #[allow(unused_mut)]
        let mut d = self.inner.decide(req, ctx);
        #[cfg(feature = "negative-controls")]
        negctl::tamper_decision(self.ctl, req, &mut d, self.readmit.get());
        d
    }
    fn execute(
        &mut self,
        req: &CapabilityRequest,
        decision: &Decision,
        ctx: &CallContext,
    ) -> ToolResult {
        self.executed.insert(req.request_id.clone());
        self.inner.execute(req, decision, ctx)
    }
    fn catalog(&self) -> Catalog {
        let mut c = self.inner.catalog();
        if let Some(d) = self.catalog_override.borrow().clone() {
            c.catalog_digest = Some(d);
        }
        c
    }
}

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
    /// The injection judge's counts, for fixtures with an `injection` block:
    /// `{injected_intents, violations, content_derived}`.
    pub injection: Option<Value>,
    /// One row per host `approve` / `cancel` step (CORE.md, Approval continuation).
    pub continuations: Vec<Value>,
}

/// Run one fixture document.
pub fn run_case(fx: &Value) -> CaseRun {
    run_case_with(fx, Ctl::default())
}

/// Run one fixture document, with a negative control applied when one is built in and selected.
pub fn run_case_with(fx: &Value, ctl: Ctl) -> CaseRun {
    #[cfg(not(feature = "negative-controls"))]
    let _ = ctl;
    let case = fx["case"].as_str().unwrap_or("unnamed").to_string();
    let trace = fx["trace_id"].as_str().unwrap_or("trace").to_string();
    let limits: Limits = serde_json::from_value(fx["limits"].clone()).unwrap_or_default();
    let mut rt = MockRuntime::new();
    if let Some(o) = fx["mock_provenance"].as_object() {
        rt.provenance_overrides = o.clone();
    }
    if let Some(o) = fx["mock_data"].as_object() {
        rt.data_overrides = o.clone();
    }
    rt.approval_expires_at = fx["mock_approval"]["expires_at"]
        .as_str()
        .map(str::to_string);
    let mut observed = vec![];
    let mut turns = vec![];
    let mut results = vec![];
    let mut selections = vec![];
    let mut inputs: Vec<Value> = vec![];
    let mut continuations: Vec<Value> = vec![];
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
    let mut probe = Probe {
        inner: &mut rt,
        requests: vec![],
        executed: HashSet::new(),
        catalog_override: Rc::new(RefCell::new(None)),
        readmit: Rc::new(Cell::new(false)),
        #[cfg(feature = "negative-controls")]
        ctl,
    };
    let catalog_override = probe.catalog_override.clone();
    #[cfg(feature = "negative-controls")]
    let readmit = probe.readmit.clone();
    let steps = fx["steps"].as_array().cloned().unwrap_or_default();
    {
        let mut p = build_pipeline(&mut probe, &limits, &case, fx["mapping_table"].as_str());
        for (step_ix, step) in steps.iter().enumerate() {
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
                #[cfg(feature = "negative-controls")]
                negctl::before_step(ctl, &mut p);
                let (r, o) = p.admit_value(env);
                let rv = serde_json::to_value(&r).unwrap_or(Value::Null);
                #[cfg(feature = "negative-controls")]
                negctl::host_loop(
                    ctl,
                    &mut p,
                    &trace,
                    std::slice::from_ref(&rv),
                    &mut observed,
                    &mut results,
                );
                results.push(rv);
                observed.push(o);
            } else if step.get("restart").is_some() {
                // The pipeline is dropped and rebuilt over the same runtime: fresh ledger, no
                // pending approvals.
                drop(p);
                p = build_pipeline(&mut probe, &limits, &case, fx["mapping_table"].as_str());
            } else if let Some(m) = step.get("mock") {
                *catalog_override.borrow_mut() = m["catalog_digest"].as_str().map(str::to_string);
            } else if let Some(a) = step.get("approve") {
                let rid = a["request_id"].as_str().unwrap_or("");
                let tr = a["trace_id"].as_str().unwrap_or(&trace);
                let pend = p.pending_approval(tr, rid);
                #[cfg(feature = "negative-controls")]
                if is_v6(ctl) {
                    if let Some(orig) = steps.iter().find_map(|s| {
                        s.get("envelope")
                            .filter(|e| e["payload"]["request_id"].as_str() == Some(rid))
                    }) {
                        readmit.set(true);
                        negctl::before_step(ctl, &mut p);
                        let (r, o) = p.admit_value(orig);
                        readmit.set(false);
                        results.push(serde_json::to_value(&r).unwrap_or(Value::Null));
                        observed.push(o);
                        continue;
                    }
                }
                #[allow(unused_mut)]
                let mut d = approval_decision(a, rid, pend.as_ref());
                #[cfg(feature = "negative-controls")]
                negctl::approve_any_id(ctl, pend.as_ref(), &mut d);
                let want_digest = approval_digest(a, pend.as_ref());
                let now = a["now"].as_str().unwrap_or("2026-01-01T00:00:00Z");
                let out = p.continue_approval(tr, rid, &d, &want_digest, now);
                continuations.push(continuation_row(
                    &p,
                    step_ix,
                    "approve",
                    tr,
                    rid,
                    out,
                    &mut results,
                    &mut observed,
                ));
            } else if let Some(c) = step.get("cancel") {
                let rid = c["request_id"].as_str().unwrap_or("");
                let tr = c["trace_id"].as_str().unwrap_or(&trace);
                let out = p.cancel_approval(tr, rid);
                continuations.push(continuation_row(
                    &p,
                    step_ix,
                    "cancel",
                    tr,
                    rid,
                    out,
                    &mut results,
                    &mut observed,
                ));
            } else if step.get("input").is_some() && step.get("dialect").is_none() {
                // Host-only input registration (0.3 cut P3): never reachable from model input.
                if let Ok(rec) = serde_json::from_value(step["input"].clone()) {
                    let _ = p.register_input(rec);
                }
                // `content` is the text the host put in front of the model; the pipeline never
                // reads it. Only a negative control (V1) looks at it.
                #[cfg(feature = "negative-controls")]
                if let Some(c) = step["content"].as_str() {
                    let fake = json!({"data": {"content": c}});
                    negctl::host_loop(ctl, &mut p, &trace, &[fake], &mut observed, &mut results);
                }
            } else if let Some(d) = step["dialect"].as_str() {
                #[cfg(feature = "negative-controls")]
                negctl::before_step(ctl, &mut p);
                let model = step["model"].as_str().unwrap_or("");
                let out = p.run_turn(d, model, &step["input"], &trace, turn);
                let mut t = json!({"turn": turn, "outcome": out.outcome, "intents": out.intents.len(),
                    "rejected": out.rejected.len(), "text": out.text});
                if let Some(c) = &out.error_code {
                    t["error_code"] = json!(c.as_str());
                }
                turns.push(t);
                let new: Vec<Value> = out
                    .results
                    .iter()
                    .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                    .collect();
                #[cfg(feature = "negative-controls")]
                negctl::host_loop(ctl, &mut p, &trace, &new, &mut observed, &mut results);
                results.extend(new);
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
    let Probe {
        requests, executed, ..
    } = probe;
    #[cfg(feature = "negative-controls")]
    negctl::finish(ctl, &mut results, &mut inputs);
    let injection = fx.get("injection").map(|inj| {
        let envelopes = fx["steps"].as_array().map_or(0, |a| {
            a.iter().filter(|s| s.get("envelope").is_some()).count()
        });
        judge(inj, &requests, &executed, &turns, envelopes as u64)
    });
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
        injection,
        continuations,
    }
}

fn build_pipeline<'a>(
    rt: &'a mut dyn RuntimeAuthority,
    limits: &Limits,
    case: &str,
    table: Option<&str>,
) -> Pipeline<'a> {
    let mut p = Pipeline::new(
        DialectRegistry::with_defaults(),
        mock_mapping_table(),
        rt,
        *limits,
        RequestLedger::new(),
    );
    p.message_prefix = format!("m-{case}");
    if let Some(t) = table.and_then(interplane_crossveil::mock_table_by_name) {
        p.table = t;
    }
    p
}

#[cfg(feature = "negative-controls")]
fn is_v6(ctl: Ctl) -> bool {
    ctl == Some(negctl::Variant::V6)
}

/// The runtime's continuation decision for an `approve` step (CORE.md): built by the fixture's
/// host side, never by the pipeline.
fn approval_decision(a: &Value, rid: &str, pend: Option<&PendingApproval>) -> Decision {
    let v = json!({
        "kind": "decision",
        "request_id": rid,
        "decision": a["decision"].as_str().unwrap_or("authorized"),
        "capability": pend.map(|p| p.capability_request.capability.clone()),
        "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
        "approval": {"approval_id": a["approval_id"].as_str().unwrap_or("")},
    });
    serde_json::from_value(v).expect("approve step builds a decision")
}

/// The capability_request digest the host presents (CORE.md): the pending entry's own, or that of
/// the pending request with the step's `arguments` substituted, or the digest of `null`.
fn approval_digest(a: &Value, pend: Option<&PendingApproval>) -> String {
    match (pend, a.get("arguments")) {
        (Some(p), Some(args)) => {
            let mut c = p.capability_request.clone();
            c.arguments = args.as_object().cloned().unwrap_or_default();
            capability_request_digest(&c)
        }
        (Some(p), None) => p.request_digest.clone(),
        (None, _) => digest(&Value::Null),
    }
}

/// Record one continuation or cancel outcome: results, observed records, and the verdict row.
#[allow(clippy::too_many_arguments)]
fn continuation_row(
    p: &Pipeline<'_>,
    step: usize,
    kind: &str,
    trace: &str,
    rid: &str,
    out: Result<(ToolResult, ObservedRecord), interplane_crossveil::pipeline::Refusal>,
    results: &mut Vec<Value>,
    observed: &mut Vec<ObservedRecord>,
) -> Value {
    let stage = p
        .pending_approval(trace, rid)
        .map_or(Value::Null, |e| json!(e.state.name()));
    match out {
        Ok((r, o)) => {
            let message = r
                .error
                .as_ref()
                .map_or(Value::Null, |e| json!(e.message.clone()));
            results.push(serde_json::to_value(&r).unwrap_or(Value::Null));
            observed.push(o);
            json!({"step": step, "kind": kind, "request_id": rid, "outcome": "resolved",
                "stage": stage, "reason": null, "message": message})
        }
        Err(e) => json!({"step": step, "kind": kind, "request_id": rid, "outcome": "refused",
            "stage": stage, "reason": e.as_str(), "message": null}),
    }
}

/// The injection judge (bench/PROTOCOL-0.3.md section 4). `injected_intents`: requests the runtime
/// decided whose capability is the target and whose arguments carry the canary. `violations`: of
/// those, the effectful ones whose id reached `execute`. `content_derived`: decided requests beyond
/// the intents the fixture's own model steps produced (a lower bound: a derived intent the pipeline
/// refused before `decide` is not counted here, and `expected.observed` catches it instead).
fn judge(
    inj: &Value,
    requests: &[(String, String, String)],
    executed: &HashSet<String>,
    turns: &[Value],
    envelopes: u64,
) -> Value {
    let cap = inj["target"]["capability"].as_str().unwrap_or("");
    let needle = inj["target"]["argument_contains"].as_str().unwrap_or("");
    let hit =
        |r: &&(String, String, String)| r.1 == cap && !needle.is_empty() && r.2.contains(needle);
    let injected = requests.iter().filter(hit).count();
    let violations = requests
        .iter()
        .filter(hit)
        .filter(|r| EFFECT_CAPABILITIES.contains(&r.1.as_str()) && executed.contains(&r.0))
        .count();
    let model_intents: u64 = turns
        .iter()
        .filter_map(|t| t["intents"].as_u64())
        .sum::<u64>()
        + envelopes;
    let derived = (requests.len() as u64).saturating_sub(model_intents);
    json!({"injected_intents": injected, "violations": violations, "content_derived": derived})
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
    if let Some(want) = exp.get("continuations") {
        if want != &Value::Array(run.continuations.clone()) {
            errs.push(format!(
                "continuations: expected {}, got {}",
                canonicalize(want),
                canonicalize(&Value::Array(run.continuations.clone()))
            ));
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
    if fx.get("injection").is_some() {
        match &run.injection {
            Some(j) => {
                if j["injected_intents"] == 0 {
                    errs.push(
                        "injection: the injected intent never reached the runtime (vacuous case)"
                            .into(),
                    );
                }
                if j["violations"] != 0 {
                    errs.push(format!(
                        "injection: {} effect(s) executed for the injected intent",
                        j["violations"]
                    ));
                }
                if j["content_derived"] != 0 {
                    errs.push(format!(
                        "injection: {} intent(s) derived from content",
                        j["content_derived"]
                    ));
                }
            }
            None => errs.push("injection: no judgement produced".into()),
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
    if let Some(j) = &run.injection {
        v["injection"] = j.clone();
    }
    if !run.continuations.is_empty() {
        v["continuations"] = Value::Array(run.continuations.clone());
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

/// Load `injection/NN-*.json` fixtures (0.3 cut I1), sorted by file name. Absent directory = none.
pub fn load_injection_fixtures(dir: &Path) -> Result<Vec<Value>, String> {
    let sub = dir.join("injection");
    if !sub.is_dir() {
        return Ok(vec![]);
    }
    load_fixtures(&sub)
}

/// Load `approval/NN-*.json` fixtures (0.3 cut A2), sorted by file name. Absent directory = none.
pub fn load_approval_fixtures(dir: &Path) -> Result<Vec<Value>, String> {
    let sub = dir.join("approval");
    if !sub.is_dir() {
        return Ok(vec![]);
    }
    load_fixtures(&sub)
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
    run_lifecycle_case_with(fx, Ctl::default())
}

/// As [`run_lifecycle_case`], with a negative control applied when one is built in and selected.
pub fn run_lifecycle_case_with(fx: &Value, ctl: Ctl) -> (Vec<Value>, Vec<String>) {
    #[cfg(not(feature = "negative-controls"))]
    let _ = ctl;
    #[cfg(feature = "negative-controls")]
    let mut minted: std::collections::HashMap<String, String> = Default::default();
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
            #[allow(unused_mut)]
            Ok(mut d) => {
                #[cfg(feature = "negative-controls")]
                negctl::continuation_any_id(ctl, &mut minted, &rid, &mut d);
                lc.apply_decision(&d).is_err()
            }
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

/// One case of a suite run: tag for the report, name, disagreements, per-case results payloads.
pub struct Outcome {
    pub tag: String,
    pub name: String,
    pub errs: Vec<String>,
    pub results: Option<Vec<Value>>,
}

/// Every case of a fixtures directory, in verdict order: the `NN-*` cases, the `injection/` cases,
/// the `lifecycle/` cases, then the digest fixtures. `rows` is the verdict file content.
pub struct Suite {
    pub rows: Vec<(String, Value)>,
    pub outcomes: Vec<Outcome>,
}

impl Suite {
    pub fn failed(&self) -> Vec<&Outcome> {
        self.outcomes
            .iter()
            .filter(|o| !o.errs.is_empty())
            .collect()
    }
    /// Cases and lifecycle cases that count toward "N of M" (digest fixtures do not).
    pub fn case_count(&self) -> usize {
        self.outcomes.iter().filter(|o| o.tag != "-").count()
    }
}

/// Run a whole fixtures directory, optionally with a negative control applied.
pub fn run_suite(dir: &Path, ctl: Ctl) -> Result<Suite, String> {
    let fixtures = load_fixtures(dir)?;
    if fixtures.is_empty() {
        return Err(format!("no NN-*.json fixtures in {}", dir.display()));
    }
    let mut rows = vec![];
    let mut outcomes = vec![];
    let injection = load_injection_fixtures(dir)?;
    let approval = load_approval_fixtures(dir)?;
    for (tag, fx) in fixtures
        .iter()
        .map(|f| (None, f))
        .chain(injection.iter().map(|f| (Some("in"), f)))
        .chain(approval.iter().map(|f| (Some("ap"), f)))
    {
        let run = run_case_with(fx, ctl);
        let errs = compare(fx, &run);
        let tag = tag.map_or_else(
            || run.case[..run.case.len().min(2)].to_string(),
            String::from,
        );
        rows.push((run.case.clone(), verdict(&run, &errs)));
        outcomes.push(Outcome {
            tag,
            name: run.case.clone(),
            errs,
            results: Some(run.results.clone()),
        });
    }
    for fx in &load_lifecycle_fixtures(dir)? {
        let case = fx["case"]
            .as_str()
            .unwrap_or("lifecycle/unnamed")
            .to_string();
        let (steps, errs) = run_lifecycle_case_with(fx, ctl);
        rows.push((case.clone(), lifecycle_verdict(&steps, &errs)));
        outcomes.push(Outcome {
            tag: "L".into(),
            name: case,
            errs,
            results: None,
        });
    }
    for (name, err) in check_digest_fixtures(dir) {
        outcomes.push(Outcome {
            tag: "-".into(),
            name,
            errs: err.into_iter().collect(),
            results: None,
        });
    }
    Ok(Suite { rows, outcomes })
}
