//! T4 runner (bench/PROTOCOL-0.3.md section 2): the AIEN subset of the 0.3 corpus through the real
//! `Pipeline` and `AienAuthority`.
//!
//! The subset and the translation of each fixture onto the adapter come from
//! `conformance/runners/adapter_subset.py --plans aien` (the rule is normative text in
//! spec/CORE.md, "Adapter subsets"); the list must equal `t4_subset` of
//! `conformance/TRUST-DIGEST.txt`. Every fixture in the subset runs. One the adapter cannot run is
//! a FAIL, never a skip (a panic inside a case is caught and reported as that case's FAIL).
//! Injection cases use the injection judgement of the conformance runners (violations,
//! content_derived, injected_intents). Approval cases are judged against the fixture's own expected
//! rows (A-AIEN): every observed record and every continuation row (outcome, stage, reason; message
//! text excluded) must equal the mock's. Set `T4_OUT=<path>` to write the verdict JSON.
use interplane_adapter_aien::*;
use interplane_core::*;
use interplane_crossveil::{CallContext, ObservedRecord, Pipeline, RuntimeAuthority};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::rc::Rc;

type Requests = Rc<RefCell<Vec<(String, String, String)>>>;
type Executed = Rc<RefCell<HashSet<String>>>;

const EFFECTS: [&str; 4] = ["append_note", "write_file", "delete_file", "send_email"];

fn repo() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The real adapter, recording every decided request and every executed id for the judge.
struct Rec {
    inner: AienShared,
    requests: Requests,
    executed: Executed,
}

impl RuntimeAuthority for Rec {
    fn runtime_id(&self) -> &str {
        RUNTIME_ID
    }
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        let args = serde_json::to_string(&req.arguments).unwrap_or_default();
        self.requests.borrow_mut().push((
            req.request_id.as_str().to_string(),
            req.capability.clone(),
            args,
        ));
        self.inner.decide(req, ctx)
    }
    fn execute(
        &mut self,
        req: &CapabilityRequest,
        decision: &Decision,
        ctx: &CallContext,
    ) -> ToolResult {
        self.executed
            .borrow_mut()
            .insert(req.request_id.as_str().to_string());
        self.inner.execute(req, decision, ctx)
    }
    fn catalog(&self) -> Catalog {
        self.inner.catalog()
    }
}

fn plans() -> Vec<Value> {
    let script = repo().join("conformance/runners/adapter_subset.py");
    let out = Command::new("python3")
        .arg(script)
        .args(["--plans", "aien"])
        .output()
        .expect("python3 must be available to compute the T4 subset");
    assert!(
        out.status.success(),
        "adapter_subset.py failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("plans are JSON")
}

fn frozen(key: &str) -> Vec<String> {
    let text = std::fs::read_to_string(repo().join("conformance/TRUST-DIGEST.txt")).unwrap();
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))
        .map(|v| {
            v.split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `YYYY-MM-DDTHH:MM:SSZ` to epoch seconds (days-from-civil), the form AIEN's grant expiry takes.
fn epoch(s: &str) -> u64 {
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().unwrap_or(0);
    let (y, m, d) = (n(0, 4), n(5, 7), n(8, 10));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days * 86400 + n(11, 13) * 3600 + n(14, 16) * 60 + n(17, 19)) as u64
}

fn key_eq(got: &Value, want: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter(|k| got[**k] != want[**k])
        .map(|k| format!("{k}: {} != {}", got[*k], want[*k]))
        .collect()
}

fn build<'a>(rec: &'a mut Rec, shared: &AienShared, case: &str) -> Pipeline<'a> {
    let table = shared.with(|a| a.mapping_table(false));
    let mut p = Pipeline::new(
        DialectRegistry::with_defaults(),
        table,
        rec,
        Limits::default(),
        RequestLedger::new(),
    );
    p.message_prefix = format!("m-{case}");
    p
}

fn run(plan: &Value) -> Value {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("notes.txt"), "hello from the workspace\n").unwrap();
    let shared = AienShared::new(AienAuthority::new(ws.path()).unwrap());
    let (requests, executed): (Requests, Executed) = Default::default();
    let mut rec = Rec {
        inner: shared.clone(),
        requests: requests.clone(),
        executed: executed.clone(),
    };
    let trace = plan["trace_id"].as_str().unwrap().to_string();
    let case = plan["case"].as_str().unwrap().to_string();
    let mut p = build(&mut rec, &shared, &case);
    let (mut observed, mut intents, mut continuations): (Vec<ObservedRecord>, u64, Vec<Value>) =
        (vec![], 0, vec![]);
    let mut minted: std::collections::HashMap<String, String> = Default::default();
    let note = |p: &Pipeline<'_>,
                o: &ObservedRecord,
                minted: &mut std::collections::HashMap<String, String>| {
        if let Some(rid) = &o.request_id {
            if let Some(pa) = p.pending_approval(&trace, rid) {
                minted.insert(rid.clone(), pa.approval_id);
            }
        }
    };
    let (mut envelopes, mut approval_rows) = (0u64, vec![]);
    for (ix, step) in plan["steps"].as_array().unwrap().iter().enumerate() {
        if let Some(h) = step.get("host_input") {
            if let Ok(r) = serde_json::from_value::<InputRecord>(h["input"].clone()) {
                let _ = p.register_input(r);
            }
        } else if step.get("restart").is_some() {
            drop(p);
            p = build(&mut rec, &shared, &case);
        } else if let Some(env) = step.get("envelope") {
            envelopes += 1;
            let (_, o) = p.admit_value(env);
            note(&p, &o, &mut minted);
            observed.push(o);
        } else if step.get("approve").is_some() || step.get("cancel").is_some() {
            let kind = if step.get("approve").is_some() {
                "approve"
            } else {
                "cancel"
            };
            let a = &step[kind];
            let rid = a["request_id"].as_str().unwrap_or("").to_string();
            let tr = a["trace_id"].as_str().unwrap_or(&trace).to_string();
            let out = if kind == "cancel" {
                p.cancel_approval(&tr, &rid)
            } else {
                let now = a["now"].as_str().unwrap_or("2026-01-01T00:00:00Z");
                let aid = match a["approval_id"]["minted_for"].as_str() {
                    Some(r) => minted
                        .get(r)
                        .cloned()
                        .unwrap_or(format!("mock-approval-{r}")),
                    None => a["approval_id"]["literal"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                };
                let pend = p.pending_approval(&tr, &rid);
                let (mut dv, dg) = match &pend {
                    Some(pa) => {
                        let exp = plan["expires_at"]
                            .as_str()
                            .map_or(epoch(now) + 86400, epoch);
                        let grant = shared
                            .with(|x| x.issue_approval(&pa.capability_request, exp))
                            .expect("host issues a grant for the pending request");
                        let d = shared.with(|x| {
                            x.present_approval(&pa.capability_request, &grant, epoch(now))
                        });
                        let dg = match a.get("arguments") {
                            Some(args) => {
                                let mut c = pa.capability_request.clone();
                                c.arguments = args.as_object().cloned().unwrap_or_default();
                                interplane_crossveil::capability_request_digest(&c)
                            }
                            None => pa.request_digest.clone(),
                        };
                        (serde_json::to_value(&d).unwrap(), dg)
                    }
                    None => (
                        json!({"kind": "decision", "request_id": rid, "decision": "authorized",
                            "capability": null, "authority": {"runtime": "aien", "policy_engine": "aien.effects"},
                            "approval": {"approval_id": ""}}),
                        digest(&Value::Null),
                    ),
                };
                dv["approval"]["approval_id"] = json!(aid);
                if let Some(d) = a.get("decision") {
                    dv["decision"] = d.clone();
                }
                if let Some(r) = a.get("decision_request_id") {
                    dv["request_id"] = r.clone();
                }
                let d: Decision = serde_json::from_value(dv).expect("continuation decision");
                let out = p.continue_approval(&tr, &rid, &d, &dg, now);
                shared.with(|x| x.discard_unexecuted(&rid));
                out
            };
            let stage = p
                .pending_approval(&tr, &rid)
                .map_or(Value::Null, |e| json!(e.state.name()));
            continuations.push(match out {
                Ok((r, o)) => {
                    let msg = r
                        .error
                        .as_ref()
                        .map_or(Value::Null, |e| json!(e.message.clone()));
                    observed.push(o);
                    json!({"step": ix, "kind": kind, "request_id": rid, "outcome": "resolved",
                        "stage": stage, "reason": null, "message": msg})
                }
                Err(e) => json!({"step": ix, "kind": kind, "request_id": rid, "outcome": "refused",
                    "stage": stage, "reason": e.as_str(), "message": null}),
            });
        } else {
            let d = step["dialect"].as_str().unwrap();
            let out = p.run_turn(
                d,
                step["model"].as_str().unwrap_or(""),
                &step["input"],
                &trace,
                step["turn"].as_u64().unwrap_or(0),
            );
            intents += out.intents.len() as u64;
            for o in &out.records {
                note(&p, o, &mut minted);
            }
            observed.extend(out.records);
        }
    }
    drop(p);
    let mut problems: Vec<String> = vec![];
    let mut row = json!({});
    if plan["kind"] == "injection" {
        let cap = plan["target"]["capability"].as_str().unwrap();
        let needle = plan["target"]["argument_contains"].as_str().unwrap();
        let reqs = requests.borrow();
        let hit = |r: &&(String, String, String)| {
            r.1 == cap && !needle.is_empty() && r.2.contains(needle)
        };
        let injected = reqs.iter().filter(hit).count();
        let violations = reqs
            .iter()
            .filter(hit)
            .filter(|r| EFFECTS.contains(&r.1.as_str()) && executed.borrow().contains(&r.0))
            .count();
        let derived = (reqs.len() as u64).saturating_sub(intents + envelopes);
        row["injection"] = json!({"injected_intents": injected, "violations": violations, "content_derived": derived});
        let decisions: std::collections::BTreeSet<String> = observed
            .iter()
            .filter(|o| {
                o.request_id
                    .as_ref()
                    .is_some_and(|id| reqs.iter().filter(hit).any(|r| &r.0 == id))
            })
            .filter_map(|o| o.decision.clone())
            .collect();
        row["injected_decisions"] = json!(decisions);
        if injected == 0 {
            problems.push("the injected intent never reached the runtime (vacuous case)".into());
        }
        if violations > 0 {
            problems.push(format!(
                "{violations} effect(s) executed for the injected intent"
            ));
        }
        if derived > 0 {
            problems.push(format!("{derived} intent(s) derived from content"));
        }
    } else {
        let want = plan["expected"]["observed"].as_array().unwrap();
        let got: Vec<Value> = observed
            .iter()
            .map(|o| serde_json::to_value(o).unwrap())
            .collect();
        if want.len() != got.len() {
            problems.push(format!("observed count {} != {}", got.len(), want.len()));
        }
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            for m in key_eq(
                g,
                w,
                &[
                    "request_id",
                    "stage",
                    "decision",
                    "status",
                    "error_code",
                    "decide_invoked",
                    "execute_invoked",
                ],
            ) {
                problems.push(format!("observed[{i}].{m}"));
            }
        }
        let wc = plan["expected"]["continuations"].as_array().unwrap();
        if wc.len() != continuations.len() {
            problems.push(format!(
                "continuations {} != {}",
                continuations.len(),
                wc.len()
            ));
        }
        for (i, (g, w)) in continuations.iter().zip(wc).enumerate() {
            for m in key_eq(
                g,
                w,
                &["step", "kind", "request_id", "outcome", "stage", "reason"],
            ) {
                problems.push(format!("continuations[{i}].{m}"));
            }
        }
        approval_rows = continuations.clone();
    }
    row["effects_executed"] = json!(executed.borrow().len());
    row["observed"] = serde_json::to_value(&observed).unwrap();
    row["continuations"] = json!(approval_rows);
    row["pass"] = json!(problems.is_empty());
    row["problems"] = json!(problems);
    row
}

#[test]
fn t4_subset_runs_and_every_case_passes() {
    let plans = plans();
    let names: Vec<String> = plans
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        frozen("t4_subset"),
        "computed T4 subset differs from conformance/TRUST-DIGEST.txt"
    );
    assert!(!names.is_empty());
    let mut rows = serde_json::Map::new();
    for plan in &plans {
        let name = plan["name"].as_str().unwrap().to_string();
        let row = catch_unwind(AssertUnwindSafe(|| run(plan))).unwrap_or_else(|e| {
            let m = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            json!({"pass": false, "problems": [format!("runner could not run it: {m}")]})
        });
        rows.insert(name, row);
    }
    let failed: Vec<&String> = rows
        .iter()
        .filter(|(_, r)| r["pass"] != true)
        .map(|(n, _)| n)
        .collect();
    let sum = |k: &str| -> u64 {
        rows.values()
            .filter_map(|r| r["injection"][k].as_u64())
            .sum()
    };
    let summary = json!({"system": "T4", "cases": rows.len(), "passed": rows.len() - failed.len(), "failed": failed,
        "violations": sum("violations"), "content_derived": sum("content_derived")});
    if let Ok(out) = std::env::var("T4_OUT") {
        let doc = json!({"summary": summary, "rows": rows});
        std::fs::write(out, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    }
    println!("T4: {summary}");
    for n in &failed {
        println!("FAIL {n}: {}", rows[*n]["problems"]);
    }
    assert!(failed.is_empty(), "T4 failures: {failed:?}");
}
