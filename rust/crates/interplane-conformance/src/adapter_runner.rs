//! Run a new adapter through its computed subset and judge it (spec/CORE.md, "Adding an adapter").
//!
//! An adapter implements [`AdapterHost`]: its runtime and policy names, a fresh authority per case
//! (`no_exposure_check` drops the authority's exposure floor check: the negative control) and the
//! pipeline it runs behind. The table entry is added to an in-memory copy of
//! `conformance/adapter-translation.json`, which is never written, so TRUST-DIGEST is untouched.
//!
//! Approval cases are judged by the A-AIEN rule (the adapter supports approval continuation). The
//! judge is the one the independent reproduction wrote from the spec text
//! (bench/runs/adapter-repro-20261005T1215Z), with the adapter-specific names taken from the host
//! and the table entry. [`run`] returns the per-case verdicts and a summary; `pass` holds when
//! every case passes. `tests/adapter_runner.rs` reproduces both recorded adapter runs (see there).
use std::collections::{HashMap, HashSet};
use std::path::Path;

use interplane_core::{digest, CapabilityRequest, Catalog, Decision, InputRecord, ToolResult};
use interplane_crossveil::{
    capability_request_digest, CallContext, ObservedRecord, Pipeline, RuntimeAuthority,
};
use regex::Regex;
use serde_json::{json, Value};

use crate::ojson::{dumps, obj, OJson, Style};
use crate::subset::Corpus;

/// What an adapter supplies to be judged.
pub trait AdapterHost {
    /// The authority's runtime id (`authority.runtime` in its decisions).
    fn runtime(&self) -> &str;
    /// The authority's policy engine name.
    fn policy(&self) -> &str;
    /// A fresh authority for one case. `expires_at` is the case's approval expiry, if any.
    fn make_authority(
        &self,
        no_exposure_check: bool,
        expires_at: Option<&str>,
    ) -> Box<dyn RuntimeAuthority>;
    /// The pipeline the adapter runs behind, over `authority` (called again on a restart step).
    fn make_pipeline<'a>(&self, authority: &'a mut dyn RuntimeAuthority) -> Pipeline<'a>;
}

/// Wraps the host authority and keeps its callback evidence, including attempted effects.
struct Recorder {
    inner: Box<dyn RuntimeAuthority>,
    decided: Vec<(String, String, Value)>,
    executed: Vec<(String, String, Value)>,
}

impl RuntimeAuthority for Recorder {
    fn runtime_id(&self) -> &str {
        self.inner.runtime_id()
    }
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        self.decided.push((
            req.request_id.clone(),
            req.capability.clone(),
            Value::Object(req.arguments.clone()),
        ));
        self.inner.decide(req, ctx)
    }
    fn execute(&mut self, req: &CapabilityRequest, d: &Decision, ctx: &CallContext) -> ToolResult {
        self.executed.push((
            req.request_id.clone(),
            req.capability.clone(),
            Value::Object(req.arguments.clone()),
        ));
        self.inner.execute(req, d, ctx)
    }
    fn catalog(&self) -> Catalog {
        self.inner.catalog()
    }
}

fn public_observed(o: &ObservedRecord) -> OJson {
    let v = serde_json::to_value(o).expect("observed record serializes");
    OJson::Obj(
        [
            "request_id",
            "stage",
            "decision",
            "status",
            "error_code",
            "decide_invoked",
            "execute_invoked",
            "result_digest",
        ]
        .iter()
        .map(|k| (k.to_string(), OJson::from_value(&v[*k])))
        .collect(),
    )
}

/// IDs of calls explicitly emitted by model steps, before host deliveries.
fn model_request_ids(plan: &OJson) -> HashSet<String> {
    let qwen =
        Regex::new(r"(?s)<tool_call>\s*<function=(\w+)>(.*?)</function>\s*</tool_call>").unwrap();
    let trace = plan.get("trace_id").as_str().unwrap_or("");
    let mut ids = HashSet::new();
    for step in plan.get("steps").as_arr() {
        let turn = dumps(step.get("turn"), Style::PLAIN);
        if step.has("envelope") {
            if let Some(id) = step
                .get("envelope")
                .get("payload")
                .get("request_id")
                .as_str()
            {
                ids.insert(id.to_string());
            }
        } else if step.get("dialect").as_str() == Some("openai") {
            for (i, call) in step
                .get("input")
                .get("tool_calls")
                .as_arr()
                .iter()
                .enumerate()
            {
                let id = call.get("id");
                ids.insert(match id.as_str() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => format!("{trace}:t{turn}:c{i}"),
                });
            }
        } else if step.get("dialect").as_str() == Some("qwen35") {
            let n = qwen
                .find_iter(step.get("input").as_str().unwrap_or(""))
                .count();
            ids.extend((0..n).map(|i| format!("{trace}:t{turn}:c{i}")));
        }
    }
    ids
}

fn opt(s: Option<&str>) -> OJson {
    s.map_or(OJson::Null, |s| s.into())
}

struct Adapter<'h, H: AdapterHost + ?Sized> {
    name: String,
    host: &'h H,
    approval_capability: Option<String>,
}

/// `v[key]`, or the error text Python's `KeyError` gave.
fn need<'a>(v: &'a OJson, key: &str) -> Result<&'a OJson, String> {
    if v.has(key) {
        Ok(v.get(key))
    } else {
        Err(format!("KeyError: '{key}'"))
    }
}

fn run_case<H: AdapterHost + ?Sized>(
    plan: &OJson,
    fixture: &OJson,
    adapter: &Adapter<'_, H>,
    no_exposure_check: bool,
) -> Result<OJson, String> {
    let host = adapter.host;
    let mut rec = Recorder {
        inner: host.make_authority(no_exposure_check, plan.get("expires_at").as_str()),
        decided: vec![],
        executed: vec![],
    };
    let trace = plan.get("trace_id").as_str().unwrap_or("").to_string();
    let explicit_ids = model_request_ids(plan);
    let mut observed: Vec<OJson> = vec![];
    let mut continuations: Vec<OJson> = vec![];
    let mut minted: HashMap<String, String> = HashMap::new();
    let mut stages: HashMap<String, String> = HashMap::new();
    {
        let mut pipe = host.make_pipeline(&mut rec);
        let note = |pipe: &Pipeline<'_>,
                    o: &ObservedRecord,
                    stages: &mut HashMap<String, String>,
                    minted: &mut HashMap<String, String>| {
            if let Some(rid) = &o.request_id {
                stages.insert(rid.clone(), o.stage.clone());
                if let Some(p) = pipe.pending_approval(&trace, rid) {
                    minted.insert(rid.clone(), p.approval_id);
                }
            }
        };
        for (index, step) in plan.get("steps").as_arr().iter().enumerate() {
            if let Some(h) = step.get_opt("host_input") {
                let r: InputRecord = serde_json::from_value(h.get("input").to_value())
                    .map_err(|e| format!("ValueError: {e}"))?;
                pipe.register_input(r)
                    .map_err(|e| format!("ValueError: {e}"))?;
            } else if step.has("dialect") {
                let turn = match step.get("turn") {
                    OJson::Int(i) => *i as u64,
                    _ => 0,
                };
                let out = pipe.run_turn(
                    step.get("dialect").as_str().unwrap_or(""),
                    step.get("model").as_str().unwrap_or(""),
                    &step.get("input").to_value(),
                    &trace,
                    turn,
                );
                for o in &out.records {
                    observed.push(public_observed(o));
                    note(&pipe, o, &mut stages, &mut minted);
                }
            } else if step.has("envelope") {
                let (_, o) = pipe.admit_value(&step.get("envelope").to_value());
                observed.push(public_observed(&o));
                note(&pipe, &o, &mut stages, &mut minted);
            } else if step.has("restart") {
                drop(pipe);
                pipe = host.make_pipeline(&mut rec);
                stages.clear();
            } else if step.has("approve") || step.has("cancel") {
                let kind = if step.has("approve") {
                    "approve"
                } else {
                    "cancel"
                };
                let action = step.get(kind);
                let rid = action.get("request_id").as_str().unwrap_or("").to_string();
                let tr = action
                    .get("trace_id")
                    .as_str()
                    .unwrap_or(&trace)
                    .to_string();
                let out = if kind == "cancel" {
                    pipe.cancel_approval(&tr, &rid)
                } else {
                    let pending = pipe.pending_approval(&tr, &rid);
                    let spec = action.get("approval_id");
                    let approval_id = match spec.get("minted_for").as_str() {
                        Some(owner) => minted
                            .get(owner)
                            .cloned()
                            .unwrap_or(format!("{}-approval-{owner}", adapter.name)),
                        None => spec.get("literal").as_str().unwrap_or("").to_string(),
                    };
                    let (request_digest, capability) = match &pending {
                        Some(p) => {
                            let d = match action.get_opt("arguments") {
                                Some(args) => {
                                    let mut changed = p.capability_request.clone();
                                    changed.arguments =
                                        args.to_value().as_object().cloned().unwrap_or_default();
                                    capability_request_digest(&changed)
                                }
                                None => p.request_digest.clone(),
                            };
                            (d, Some(p.capability_request.capability.clone()))
                        }
                        None => (digest(&Value::Null), adapter.approval_capability.clone()),
                    };
                    let decision: Decision = serde_json::from_value(json!({
                        "kind": "decision",
                        "request_id": action.get_opt("decision_request_id").map_or(json!(rid), OJson::to_value),
                        "decision": action.get_opt("decision").map_or(json!("authorized"), OJson::to_value),
                        "capability": capability,
                        "authority": {"runtime": host.runtime(), "policy_engine": host.policy(),
                                      "decision_id": format!("{}-host-grant-{rid}", adapter.name)},
                        "approval": {"approval_id": approval_id, "scope": "single_action",
                                     "expires_at": pending.as_ref().and_then(|p| p.expires_at.clone())},
                    }))
                    .map_err(|e| format!("ValueError: {e}"))?;
                    let now = action
                        .get("now")
                        .as_str()
                        .unwrap_or("2026-01-01T00:00:00Z")
                        .to_string();
                    pipe.continue_approval(&tr, &rid, &decision, &request_digest, &now)
                };
                match out {
                    Ok((result, o)) => {
                        observed.push(public_observed(&o));
                        stages.insert(rid.clone(), o.stage.clone());
                        continuations.push(obj([
                            ("step", OJson::Int(index as i128)),
                            ("kind", kind.into()),
                            ("request_id", rid.into()),
                            ("outcome", "resolved".into()),
                            ("stage", o.stage.clone().into()),
                            ("reason", OJson::Null),
                            (
                                "message",
                                opt(result.error.as_ref().map(|e| e.message.as_str())),
                            ),
                        ]));
                    }
                    Err(refusal) => {
                        let stage = opt(stages.get(&rid).map(String::as_str));
                        continuations.push(obj([
                            ("step", OJson::Int(index as i128)),
                            ("kind", kind.into()),
                            ("request_id", rid.into()),
                            ("outcome", "refused".into()),
                            ("stage", stage),
                            ("reason", refusal.as_str().into()),
                            ("message", OJson::Null),
                        ]));
                    }
                }
            } else {
                return Err(format!(
                    "ValueError: untranslated step at index {index}: {:?}",
                    step.keys()
                ));
            }
        }
    }
    let decide_calls = rec.decided.len() as i128;
    let execute_calls = rec.executed.len() as i128;
    let mut row = obj([
        ("name", plan.get("name").clone()),
        ("pass", OJson::Bool(false)),
        ("executed", OJson::Bool(true)),
        ("observed", OJson::Arr(observed.clone())),
    ]);
    if plan.get("kind").as_str() == Some("injection") {
        let target = plan.get("target").get("capability").as_str().unwrap_or("");
        let canary = plan
            .get("target")
            .get("argument_contains")
            .as_str()
            .unwrap_or("");
        let injected = |e: &&(String, String, Value)| {
            e.1 == target && dumps(&OJson::from_value(&e.2), Style::SORTED).contains(canary)
        };
        let intents = rec.decided.iter().filter(injected).count() as i128;
        let violations = rec.executed.iter().filter(injected).count() as i128;
        let derived = rec
            .decided
            .iter()
            .filter(|(rid, _, _)| !explicit_ids.contains(rid))
            .count() as i128;
        row.set(
            "injection",
            obj([
                ("injected_intents", OJson::Int(intents)),
                ("violations", OJson::Int(violations)),
                ("content_derived", OJson::Int(derived)),
            ]),
        );
        row.set(
            "pass",
            OJson::Bool(intents > 0 && violations == 0 && derived == 0),
        );
    } else {
        // A missing key is an error row, as the Python runner gave (`KeyError: 'runtime'`).
        let expected = need(fixture, "expected")?;
        let want_runtime = need(expected, "runtime")?.to_value();
        let left: Vec<Value> = observed
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.set("result_digest", OJson::Null);
                r.to_value()
            })
            .collect();
        let want: Vec<Value> = need(expected, "observed")?
            .as_arr()
            .iter()
            .map(OJson::to_value)
            .collect();
        let keys = ["step", "kind", "request_id", "outcome", "stage", "reason"];
        let pick = |rows: &[OJson]| -> Vec<Value> {
            rows.iter()
                .map(|r| {
                    Value::Object(
                        keys.iter()
                            .map(|k| (k.to_string(), r.get(k).to_value()))
                            .collect(),
                    )
                })
                .collect()
        };
        let runtime = json!({"decide_calls": decide_calls, "execute_calls": execute_calls});
        let checks = obj([
            ("observed", OJson::Bool(left == want)),
            (
                "continuations",
                OJson::Bool(pick(&continuations) == pick(expected.get("continuations").as_arr())),
            ),
            ("runtime", OJson::Bool(runtime == want_runtime)),
        ]);
        let all = checks
            .entries()
            .iter()
            .all(|(_, v)| *v == OJson::Bool(true));
        row.set("continuations", OJson::Arr(continuations));
        row.set("checks", checks);
        row.set("pass", OJson::Bool(all));
    }
    Ok(row)
}

/// Per-case verdicts and the summary of one adapter run.
#[derive(Debug, Clone)]
pub struct Verdicts {
    pub adapter: String,
    /// `{"rows": [...], "summary": {...}}`
    pub value: OJson,
}

impl Verdicts {
    pub fn summary(&self) -> &OJson {
        self.value.get("summary")
    }

    /// True when every case in the subset passed.
    pub fn passed(&self) -> bool {
        matches!(self.summary().get("failed"), OJson::Int(0))
    }

    /// The verdict file: indented, keys sorted, one trailing newline.
    pub fn file_text(&self) -> String {
        dumps(&self.value, Style::SORTED_INDENT2) + "\n"
    }
}

/// Run `host` through its subset. `entry` adds `{name: entry}` to an in-memory copy of the shared
/// table; `None` uses the adapter already in the table under `name`.
pub fn run<H: AdapterHost + ?Sized>(
    root: &Path,
    name: &str,
    entry: Option<&OJson>,
    host: &H,
    no_exposure_check: bool,
) -> Result<Verdicts, String> {
    let corpus = Corpus::new(root);
    let mut table = corpus.load_table()?;
    if let Some(e) = entry {
        let adapters = table
            .get_mut("adapters")
            .ok_or("translation table without adapters")?;
        adapters.set(name, e.clone());
    }
    let entry = table
        .get("adapters")
        .get_opt(name)
        .ok_or_else(|| format!("{name} is not in conformance/adapter-translation.json"))?
        .clone();
    let adapter = Adapter {
        name: name.to_string(),
        host,
        approval_capability: entry
            .get("approval_effects")
            .get("delete_file")
            .get("name")
            .as_str()
            .map(String::from),
    };
    let plans = corpus.plans(name, &table)?;
    let fixtures: HashMap<String, OJson> = corpus.fixtures()?.into_iter().collect();
    let mut rows = vec![];
    for plan in &plans {
        let pname = plan.get("name").as_str().unwrap_or("").to_string();
        let case = fixtures
            .get(&pname)
            .ok_or_else(|| format!("KeyError: '{pname}'"))
            .and_then(|f| run_case(plan, f, &adapter, no_exposure_check));
        let row = match case {
            Ok(r) => r,
            Err(e) => obj([
                ("name", pname.into()),
                ("pass", OJson::Bool(false)),
                ("executed", OJson::Bool(false)),
                ("error", e.into()),
            ]),
        };
        rows.push(row);
    }
    let count = |f: &dyn Fn(&OJson) -> bool| rows.iter().filter(|r| f(r)).count() as i128;
    let sum = |k: &str| -> i128 {
        rows.iter()
            .map(|r| match r.get("injection").get(k) {
                OJson::Int(i) => *i,
                _ => 0,
            })
            .sum()
    };
    let summary = obj([
        ("subset_size", OJson::Int(plans.len() as i128)),
        (
            "executed",
            OJson::Int(count(&|r| *r.get("executed") == OJson::Bool(true))),
        ),
        (
            "failed",
            OJson::Int(count(&|r| *r.get("pass") != OJson::Bool(true))),
        ),
        ("violations", OJson::Int(sum("violations"))),
        ("content_derived", OJson::Int(sum("content_derived"))),
        ("injected_intents", OJson::Int(sum("injected_intents"))),
    ]);
    Ok(Verdicts {
        adapter: name.to_string(),
        value: obj([("rows", OJson::Arr(rows)), ("summary", summary)]),
    })
}
