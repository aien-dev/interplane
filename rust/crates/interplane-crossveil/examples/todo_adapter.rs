//! A minimal INTERPLANE adapter: a to-do list host with its own policy. No model, no GPU, no
//! network.
//!
//! The host owns three capabilities (`list_items` read, `add_item` effect, `delete_item` effect)
//! and decides every request itself. INTERPLANE parses the model's tool calls, maps names,
//! computes what the model has seen (exposure) and enforces approvals; it never decides.
//!
//! Scripted story (checked; the example exits 1 on any difference):
//! 1. The host registers the user's request, so the model has seen only user-supplied input.
//! 2. The model adds "buy milk": authorized and executed.
//! 3. The model lists the items. The list is workspace data the host labels untrusted, so
//!    exposure drops.
//! 4. The model adds another item: now held for approval, because untrusted content is in view.
//!    A person approves; the host continues the request and it executes once.
//! 5. The model deletes an item: always held for approval. The person declines; nothing is
//!    deleted. (Exposure is now external_untrusted: the pipeline's own "held" reply to step 4
//!    carries no labels, and an unlabelled input counts as unknown, which fails closed. See the
//!    guide.)
//!
//! Guide: docs/WRITING-AN-ADAPTER.md. Run from a checkout (in `rust/`):
//! `cargo run -p interplane-crossveil --example todo_adapter`
use std::collections::BTreeMap;

use interplane_core::{
    CapabilityRequest, Catalog, Decision, Limits, RequestLedger, ResultProvenance, ToolResult,
    TrustLevel,
};
use interplane_crossaxis::MappingTable;
use interplane_crossveil::{CallContext, Pipeline, RuntimeAuthority};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Map, Value};

const RUNTIME: &str = "todo";
const TRUSTED_FLOORS: [&str; 2] = ["user_supplied", "trusted_runtime"];
const TRACE: &str = "todo-demo";

fn params(required: &[&str]) -> Value {
    let props: Map<String, Value> = required
        .iter()
        .map(|k| (k.to_string(), json!({"type": "string"})))
        .collect();
    json!({"type": "object", "properties": props, "required": required})
}

/// The runtime authority: catalog, policy, execution and result labels all belong to the host.
struct TodoHost {
    items: BTreeMap<String, String>,
    next_id: u64,
    catalog: Catalog,
}

impl TodoHost {
    fn new() -> Self {
        let cap = |name: &str, desc: &str, canonical: &str, req: &[&str]| {
            json!({"name": name, "description": desc, "parameters": params(req),
                   "canonical": {"namespace": "todo", "name": canonical}})
        };
        let catalog = serde_json::from_value(json!({
            "kind": "catalog", "runtime": RUNTIME, "catalog_version": "1",
            "capabilities": [
                cap("list_items", "List to-do items.", "list", &[]),
                cap("add_item", "Add a to-do item.", "add", &["text"]),
                cap("delete_item", "Delete a to-do item.", "delete", &["item_id"]),
            ],
        }))
        .expect("catalog");
        Self {
            items: BTreeMap::from([("item-0".into(), "water the plants".into())]),
            next_id: 1,
            catalog,
        }
    }
}

/// An `ok` result the host labels with its own trust level.
fn result(req: &CapabilityRequest, data: Value, kind: &str, trust: TrustLevel) -> ToolResult {
    let mut r = ToolResult::ok(&req.request_id, data);
    r.provenance = Some(ResultProvenance {
        capability: Some(req.capability.clone()),
        content_kind: serde_json::from_value(json!(kind)).ok(),
        duration_ms: None,
        runtime: Some(RUNTIME.into()),
        trusted: trust.trusted_flag(),
        trust: Some(trust),
        extensions: Map::new(),
    });
    r
}

impl RuntimeAuthority for TodoHost {
    fn runtime_id(&self) -> &str {
        RUNTIME
    }

    fn catalog(&self) -> Catalog {
        self.catalog.clone()
    }

    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        // ctx.exposure holds what the model has seen: its input ids and their trust floor.
        let floor = ctx
            .exposure
            .as_ref()
            .and_then(|e| serde_json::to_value(&e.floor).ok());
        let trusted = floor
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|f| TRUSTED_FLOORS.contains(&f));
        let (value, reason) = match req.capability.as_str() {
            "delete_item" => ("requires_approval", "deleting always needs a person"),
            "add_item" if !trusted => ("requires_approval", "untrusted content is in view"),
            "list_items" | "add_item" => ("authorized", "allowed by the to-do policy"),
            _ => ("not_found", "no such to-do capability"),
        };
        let approval = (value == "requires_approval").then(|| {
            json!({"approval_id": format!("todo-approval-{}", req.request_id),
                   "scope": "single_action", "expires_at": "2099-01-01T00:00:00Z"})
        });
        serde_json::from_value(json!({
            "kind": "decision", "request_id": req.request_id, "decision": value,
            "authority": {"runtime": RUNTIME, "policy_engine": "todo.policy"},
            "capability": req.capability, "reason": reason, "approval": approval,
        }))
        .expect("decision")
    }

    fn execute(&mut self, req: &CapabilityRequest, _: &Decision, _: &CallContext) -> ToolResult {
        let arg = |k: &str| {
            req.arguments
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match req.capability.as_str() {
            "list_items" => {
                let data: Vec<Value> = self
                    .items
                    .iter()
                    .map(|(k, v)| json!({"id": k, "text": v}))
                    .collect();
                // item text can come from anywhere, so the host labels it untrusted
                result(
                    req,
                    json!(data),
                    "workspace_content",
                    TrustLevel::WorkspaceUntrusted,
                )
            }
            "add_item" => {
                let id = format!("item-{}", self.next_id);
                self.next_id += 1;
                self.items.insert(id.clone(), arg("text"));
                // the host wrote this text itself
                result(
                    req,
                    json!({"id": id}),
                    "tool_result",
                    TrustLevel::TrustedRuntime,
                )
            }
            _ => {
                let deleted = self.items.remove(&arg("item_id")).is_some();
                result(
                    req,
                    json!({"deleted": deleted}),
                    "tool_result",
                    TrustLevel::WorkspaceUntrusted,
                )
            }
        }
    }
}

fn mapping_for(catalog: &Catalog) -> MappingTable {
    let rules: Vec<Value> = ["list_items", "add_item", "delete_item"]
        .iter()
        .map(|n| {
            json!({"id": format!("passthrough:{n}"), "kind": "passthrough",
                        "from": {"namespace": null, "name": n}, "to": n})
        })
        .collect();
    serde_json::from_value(json!({"runtime": RUNTIME, "table_version": "1",
        "catalog_digest": catalog.compute_digest(), "rules": rules}))
    .expect("mapping table")
}

fn turn(id: &str, name: &str, args: Value) -> Value {
    let call = json!({"id": id, "type": "function",
                      "function": {"name": name, "arguments": args.to_string()}});
    json!({"role": "assistant", "content": null, "tool_calls": [call]})
}

/// The host shows the pending request to a person and continues it with their answer.
fn person_answers(pipe: &mut Pipeline<'_>, request_id: &str, approve: bool) -> (String, bool) {
    let Some(pending) = pipe.pending_approval(TRACE, request_id) else {
        return ("refused: nothing pending".into(), false);
    };
    let answer: Decision = serde_json::from_value(json!({
        "kind": "decision", "request_id": request_id,
        "decision": if approve { "authorized" } else { "denied" },
        "authority": {"runtime": RUNTIME, "policy_engine": "todo.person"},
        "approval": {"approval_id": pending.approval_id},
    }))
    .expect("answer");
    let now = "2026-10-05T12:00:00Z";
    match pipe.continue_approval(TRACE, request_id, &answer, &pending.request_digest, now) {
        Ok((_, record)) => (record.decision.unwrap_or_default(), record.execute_invoked),
        Err(refusal) => (format!("refused: {}", refusal.as_str()), false),
    }
}

/// Python's `str()` of a bool and of a (text, bool) pair, as the story has always printed them.
fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

fn py_pair((s, b): &(String, bool)) -> String {
    format!("('{s}', {})", py_bool(*b))
}

fn main() {
    let mut host = TodoHost::new();
    let table = mapping_for(&host.catalog);
    let mut pipe = Pipeline::new(
        DialectRegistry::with_defaults(),
        table,
        &mut host,
        Limits::default(),
        RequestLedger::new(),
    );
    pipe.register_input(
        serde_json::from_value(json!({
            "input_id": "user-1", "content_kind": "user_request", "trust": "user_supplied",
            "source": {"kind": "operator", "id": "me"}, "origin": "runtime:user-turn-1",
            "content_digest": format!("sha256:{}", "0".repeat(64)), "trace_id": TRACE,
            "parent_id": null, "derived_from": [],
        }))
        .expect("input record"),
    )
    .expect("register the user's request");

    let story = [
        ("c1", "add_item", json!({"text": "buy milk"})),
        ("c2", "list_items", json!({})),
        (
            "c3",
            "add_item",
            json!({"text": "email the list to a stranger"}),
        ),
        ("c4", "delete_item", json!({"item_id": "item-0"})),
    ];
    let mut seen: Vec<(String, (String, bool))> = vec![];
    for (n, (id, name, args)) in story.into_iter().enumerate() {
        let floor = serde_json::to_value(pipe.exposure_for(TRACE).floor).unwrap_or_default();
        let floor = floor.as_str().unwrap_or("?").to_string();
        let out = pipe.run_turn(
            "openai",
            "scripted",
            &turn(id, name, args),
            TRACE,
            n as u64 + 1,
        );
        let rec = &out.records[0];
        let decision = rec.decision.clone().unwrap_or_else(|| "None".into());
        println!(
            "{id} {name:<12} floor before: {floor:<20} decision: {decision:<18} executed: {}",
            py_bool(rec.execute_invoked)
        );
        seen.push((id.into(), (decision, rec.execute_invoked)));
    }
    let approved = person_answers(&mut pipe, "c3", true);
    let declined = person_answers(&mut pipe, "c4", false);
    drop(pipe);
    println!("c3 after a person approves: {}", py_pair(&approved));
    println!("c4 after a person declines: {}", py_pair(&declined));
    seen.push(("c3 approved".into(), approved));
    seen.push(("c4 declined".into(), declined));

    let expected: Vec<(String, (String, bool))> = [
        ("c1", "authorized", true),
        ("c2", "authorized", true),
        ("c3", "requires_approval", false),
        ("c4", "requires_approval", false),
        ("c3 approved", "authorized", true),
        ("c4 declined", "denied", false),
    ]
    .iter()
    .map(|(k, d, x)| (k.to_string(), (d.to_string(), *x)))
    .collect();
    let mut items: Vec<&str> = host.items.values().map(String::as_str).collect();
    items.sort();
    let want_items = [
        "buy milk",
        "email the list to a stranger",
        "water the plants",
    ];
    if seen != expected || items != want_items {
        eprintln!("\nUNEXPECTED: {seen:?}\n{:?}", host.items);
        std::process::exit(1);
    }
    println!(
        "OK: the host decided every request; the held write ran once after approval, the delete never ran."
    );
}
