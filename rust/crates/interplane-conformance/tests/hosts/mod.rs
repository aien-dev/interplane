//! The two hosts whose adapter runs are recorded under bench/runs, as [`AdapterHost`]s:
//!
//! * `toy`: the to-do host of the independent reproduction (bench/runs/adapter-repro-20261005T1215Z).
//! * `home`: the maintainer's smart-home check (bench/runs/home-check-20261005T1320Z).
//!
//! Each is a line-for-line port of the recorded host; `tests/adapter_runner.rs` proves it against
//! the recorded verdict files.
#![allow(dead_code)]
use interplane_conformance::adapter_runner::AdapterHost;
use interplane_core::{
    CapabilityDescriptor, CapabilityRequest, Catalog, Decision, Limits, RequestLedger,
    ResultProvenance, ToolResult, TrustLevel,
};
use interplane_crossaxis::MappingTable;
use interplane_crossveil::{CallContext, Pipeline, RuntimeAuthority};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Map, Value};

const TRUSTED_FLOORS: [&str; 2] = ["user_supplied", "trusted_runtime"];

/// `{"type": "object", "properties": {k: {"type": "string"}}, "required": [...]}`
fn params(required: &[&str], optional: &[&str]) -> Map<String, Value> {
    let props: Map<String, Value> = required
        .iter()
        .chain(optional)
        .map(|k| (k.to_string(), json!({"type": "string"})))
        .collect();
    json!({"type": "object", "properties": props, "required": required})
        .as_object()
        .unwrap()
        .clone()
}

fn descriptor(
    name: &str,
    desc: &str,
    ns: &str,
    canonical: &str,
    p: Map<String, Value>,
    domains: &[&str],
) -> CapabilityDescriptor {
    serde_json::from_value(json!({
        "name": name, "description": desc, "parameters": p, "domains": domains,
        "canonical": {"namespace": ns, "name": canonical},
    }))
    .unwrap()
}

fn catalog(runtime: &str, caps: Vec<CapabilityDescriptor>) -> Catalog {
    serde_json::from_value(json!({"kind": "catalog", "runtime": runtime, "catalog_version": "1", "capabilities": caps}))
        .unwrap()
}

/// A mapping table: one passthrough rule per name, then `(namespace, alias, target)` alias rules.
fn table(
    runtime: &str,
    cat: &Catalog,
    names: &[&str],
    aliases: &[(&str, &str, &str)],
) -> MappingTable {
    let mut rules: Vec<Value> = names
        .iter()
        .map(|n| json!({"id": format!("passthrough:{n}"), "kind": "passthrough", "from": {"namespace": null, "name": n}, "to": n}))
        .collect();
    rules.extend(aliases.iter().map(|(ns, a, t)| {
        json!({"id": format!("alias:{ns}.{a}"), "kind": "alias", "from": {"namespace": ns, "name": a}, "to": t})
    }));
    serde_json::from_value(json!({"runtime": runtime, "table_version": "1",
        "catalog_digest": cat.compute_digest(), "rules": rules}))
    .unwrap()
}

fn pipeline<'a>(table: MappingTable, authority: &'a mut dyn RuntimeAuthority) -> Pipeline<'a> {
    Pipeline::new(
        DialectRegistry::with_defaults(),
        table,
        authority,
        Limits::default(),
        RequestLedger::new(),
    )
}

/// The decision a host returns: its own runtime, policy and decision id.
#[allow(clippy::too_many_arguments)]
fn decision(
    rid: &str,
    value: &str,
    runtime: &str,
    policy: &str,
    decision_id: &str,
    cap: &str,
    reason: &str,
    approval: Value,
) -> Decision {
    serde_json::from_value(json!({
        "kind": "decision", "request_id": rid, "decision": value, "capability": cap,
        "authority": {"runtime": runtime, "policy_engine": policy, "decision_id": decision_id},
        "reason": reason, "approval": approval,
    }))
    .unwrap()
}

fn approval(id: String, expires_at: &Option<String>) -> Value {
    json!({"approval_id": id, "scope": "single_action", "expires_at": expires_at})
}

/// An `ok` result labelled by the host (`trusted` follows `trust`).
fn labelled(
    rid: &str,
    runtime: &str,
    cap: &str,
    data: Value,
    kind: &str,
    trust: &str,
) -> ToolResult {
    let mut r = ToolResult::ok(rid, data);
    let trust: TrustLevel = serde_json::from_value(json!(trust)).unwrap();
    r.provenance = Some(ResultProvenance {
        capability: Some(cap.into()),
        content_kind: serde_json::from_value(json!(kind)).unwrap(),
        duration_ms: None,
        runtime: Some(runtime.into()),
        trusted: trust.trusted_flag(),
        trust: Some(trust),
        extensions: Map::new(),
    });
    r
}

fn floor(ctx: &CallContext) -> Option<String> {
    ctx.exposure
        .as_ref()
        .and_then(|e| serde_json::to_value(&e.floor).ok())
        .and_then(|v| v.as_str().map(String::from))
}

fn text_args(args: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|k| args.get(*k).is_some_and(Value::is_string))
}

fn arg(args: &Map<String, Value>, k: &str) -> String {
    args.get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

// ---- smart-home host (bench/runs/home-check-20261005T1320Z)

pub struct Home;

const HOME_NAMES: [&str; 4] = ["read_sensors", "set_light", "send_alert", "unlock_door"];

fn home_catalog() -> Catalog {
    let cap = |n: &str, d: &str, req: &[&str], opt: &[&str]| {
        descriptor(n, d, "home", n, params(req, opt), &["home"])
    };
    catalog(
        "home",
        vec![
            cap("read_sensors", "Read the home's sensors.", &[], &["room"]),
            cap("set_light", "Set a room's light.", &["room", "level"], &[]),
            cap(
                "send_alert",
                "Send an alert message out of the home.",
                &["to", "message"],
                &[],
            ),
            cap("unlock_door", "Unlock a door.", &["door"], &[]),
        ],
    )
}

pub struct HomeAuthority {
    no_exposure_check: bool,
    expires_at: Option<String>,
    catalog: Catalog,
    pub lights: Vec<(String, String)>,
    pub alerts: Vec<(String, String)>,
    pub unlocked: Vec<String>,
}

impl RuntimeAuthority for HomeAuthority {
    fn runtime_id(&self) -> &str {
        "home"
    }
    fn catalog(&self) -> Catalog {
        self.catalog.clone()
    }
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        let trusted = self.no_exposure_check
            || floor(ctx).is_some_and(|f| TRUSTED_FLOORS.contains(&f.as_str()));
        let required: &[&str] = match req.capability.as_str() {
            "read_sensors" => &[],
            "set_light" => &["room", "level"],
            "send_alert" => &["to", "message"],
            "unlock_door" => &["door"],
            _ => &["\u{0}unknown"],
        };
        let (value, reason) = if !HOME_NAMES.contains(&req.capability.as_str()) {
            ("not_found", "no such home capability")
        } else if !text_args(&req.arguments, required) {
            ("invalid", "missing or non-text argument")
        } else if req.capability == "read_sensors" {
            ("authorized", "reads are allowed")
        } else if req.capability == "unlock_door" {
            (
                "requires_approval",
                "unlocking a door always needs a person",
            )
        } else if req.capability == "set_light" {
            if trusted {
                ("authorized", "allowed")
            } else {
                ("requires_approval", "untrusted content in view")
            }
        } else if trusted {
            // send_alert: data leaving the home is refused after untrusted input
            ("authorized", "allowed")
        } else {
            ("denied", "no outbound alerts after untrusted input")
        };
        let ap = if value == "requires_approval" {
            approval(
                format!("home-approval-{}", req.request_id),
                &self.expires_at,
            )
        } else {
            Value::Null
        };
        let did = format!("home-{}", req.request_id);
        decision(
            &req.request_id,
            value,
            "home",
            "home.policy",
            &did,
            &req.capability,
            reason,
            ap,
        )
    }
    fn execute(
        &mut self,
        req: &CapabilityRequest,
        _d: &Decision,
        _ctx: &CallContext,
    ) -> ToolResult {
        let a = &req.arguments;
        let rid = &req.request_id;
        match req.capability.as_str() {
            "read_sensors" => {
                let lights: Map<String, Value> = self
                    .lights
                    .iter()
                    .map(|(k, v)| (k.clone(), json!(v)))
                    .collect();
                // room names are user-set
                return labelled(
                    rid,
                    "home",
                    &req.capability,
                    json!({"lights": lights, "temperature_c": 21}),
                    "workspace_content",
                    "workspace_untrusted",
                );
            }
            "set_light" => {
                let (room, level) = (arg(a, "room"), arg(a, "level"));
                match self.lights.iter_mut().find(|(k, _)| *k == room) {
                    Some(slot) => slot.1 = level,
                    None => self.lights.push((room, level)),
                }
            }
            "send_alert" => self.alerts.push((arg(a, "to"), arg(a, "message"))),
            _ => self.unlocked.push(arg(a, "door")),
        }
        labelled(
            rid,
            "home",
            &req.capability,
            json!({"done": req.capability}),
            "tool_result",
            "trusted_runtime",
        )
    }
}

impl AdapterHost for Home {
    fn runtime(&self) -> &str {
        "home"
    }
    fn policy(&self) -> &str {
        "home.policy"
    }
    fn make_authority(
        &self,
        no_exposure_check: bool,
        expires_at: Option<&str>,
    ) -> Box<dyn RuntimeAuthority> {
        Box::new(HomeAuthority {
            no_exposure_check,
            expires_at: expires_at.map(String::from),
            catalog: home_catalog(),
            lights: vec![("hall".into(), "off".into())],
            alerts: vec![],
            unlocked: vec![],
        })
    }
    fn make_pipeline<'a>(&self, authority: &'a mut dyn RuntimeAuthority) -> Pipeline<'a> {
        let t = table("home", &authority.catalog(), &HOME_NAMES, &[]);
        pipeline(t, authority)
    }
}

// ---- to-do host of the independent reproduction (bench/runs/adapter-repro-20261005T1215Z)

pub struct Toy;

fn toy_catalog() -> Catalog {
    catalog(
        "toy",
        vec![
            descriptor(
                "list_items",
                "List to-do items.",
                "todo",
                "list",
                params(&[], &["scope"]),
                &["todo"],
            ),
            descriptor(
                "add_item",
                "Add a to-do item.",
                "todo",
                "add",
                params(&["text"], &["list"]),
                &["todo"],
            ),
            descriptor(
                "delete_item",
                "Delete a to-do item.",
                "todo",
                "delete",
                params(&["item_id"], &[]),
                &["todo"],
            ),
        ],
    )
}

pub struct TodoAuthority {
    no_exposure_check: bool,
    expires_at: Option<String>,
    catalog: Catalog,
    pub items: Vec<(String, Value)>,
    next_id: u64,
}

impl RuntimeAuthority for TodoAuthority {
    fn runtime_id(&self) -> &str {
        "toy"
    }
    fn catalog(&self) -> Catalog {
        self.catalog.clone()
    }
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        let name = req.capability.as_str();
        let args = &req.arguments;
        let (required, optional): (&[&str], &[&str]) = match name {
            "list_items" => (&[], &["scope"]),
            "add_item" => (&["text"], &["list"]),
            "delete_item" => (&["item_id"], &[]),
            _ => (&[], &[]),
        };
        let trusted_floor = floor(ctx).is_some_and(|f| TRUSTED_FLOORS.contains(&f.as_str()));
        let (value, reason) = if !["list_items", "add_item", "delete_item"].contains(&name) {
            ("not_found", "unknown to-do capability")
        } else if !text_args(args, required)
            || optional
                .iter()
                .any(|k| args.get(*k).is_some_and(|v| !v.is_string()))
        {
            ("invalid", "required to-do argument missing or not text")
        } else if name == "delete_item" {
            ("requires_approval", "toy policy: deletion needs approval")
        } else if name == "add_item" && !self.no_exposure_check && !trusted_floor {
            (
                "requires_approval",
                "toy policy: effects after untrusted input need approval",
            )
        } else {
            ("authorized", "toy policy permits this action")
        };
        let ap = if value == "requires_approval" {
            approval(format!("toy-approval-{}", req.request_id), &self.expires_at)
        } else {
            Value::Null
        };
        let did = format!("toy-decision-{}", req.request_id);
        decision(
            &req.request_id,
            value,
            "toy",
            "toy.todo.policy",
            &did,
            name,
            reason,
            ap,
        )
    }
    fn execute(
        &mut self,
        req: &CapabilityRequest,
        _d: &Decision,
        _ctx: &CallContext,
    ) -> ToolResult {
        let (name, args, rid) = (req.capability.as_str(), &req.arguments, &req.request_id);
        let (data, kind) = match name {
            "list_items" => {
                let items: Vec<Value> = self.items.iter().map(|(_, v)| v.clone()).collect();
                (json!({"items": items}), "workspace_content")
            }
            "add_item" => {
                let id = format!("item-{}", self.next_id);
                self.next_id += 1;
                let list = args.get("list").cloned().unwrap_or(json!("inbox"));
                let item = json!({"id": id, "text": args.get("text").cloned().unwrap_or(Value::Null), "list": list});
                self.items.push((id, item.clone()));
                (json!({"item": item}), "tool_result")
            }
            _ => {
                let id = arg(args, "item_id");
                let before = self.items.len();
                self.items.retain(|(k, _)| *k != id);
                (
                    json!({"item_id": id, "deleted": self.items.len() != before}),
                    "tool_result",
                )
            }
        };
        labelled(rid, "toy", name, data, kind, "workspace_untrusted")
    }
}

impl AdapterHost for Toy {
    fn runtime(&self) -> &str {
        "toy"
    }
    fn policy(&self) -> &str {
        "toy.todo.policy"
    }
    fn make_authority(
        &self,
        no_exposure_check: bool,
        expires_at: Option<&str>,
    ) -> Box<dyn RuntimeAuthority> {
        let items = ["old.txt", "a.txt", "b.txt"]
            .iter()
            .map(|k| {
                (
                    k.to_string(),
                    json!({"id": k, "text": "Existing item", "list": "inbox"}),
                )
            })
            .collect();
        Box::new(TodoAuthority {
            no_exposure_check,
            expires_at: expires_at.map(String::from),
            catalog: toy_catalog(),
            items,
            next_id: 1,
        })
    }
    fn make_pipeline<'a>(&self, authority: &'a mut dyn RuntimeAuthority) -> Pipeline<'a> {
        let names = ["list_items", "add_item", "delete_item"];
        let aliases = [
            ("todo", "list", "list_items"),
            ("todo", "add", "add_item"),
            ("todo", "delete", "delete_item"),
        ];
        let t = table("toy", &authority.catalog(), &names, &aliases);
        pipeline(t, authority)
    }
}
