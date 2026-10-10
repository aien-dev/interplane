//! Line-delimited JSON bridge over the real [`AienAuthority`].
//!
//! `aien-authority-bridge <workspace>` reads one JSON request per line on stdin and writes
//! exactly one JSON line per request on stdout. It adds no authority: `decide`, `execute`
//! and the approval calls go straight to `AienAuthority`, and every answer is the serde JSON of the
//! real `interplane-core` type (`Catalog`, `Decision`, `ToolResult`). It decides nothing itself.
//!
//! Requests (`op`):
//!   `catalog`                                   -> `{"ok":true,"catalog":Catalog}`
//!   `mapping_table`                             -> `{"ok":true,"mapping_table":MappingTable}` (pinned)
//!   `decide`   `request`, `ctx`                 -> `{"ok":true,"decision":Decision}`
//!   `execute`  `request`, `decision`, `ctx`     -> `{"ok":true,"result":ToolResult}`
//!   `approve`  `request`, optional `ttl_secs`   -> `{"ok":true,"decision":Decision}`  (host-only
//!              approver channel: AIEN's own desk issues one single-use grant for exactly this
//!              request and `present_approval` spends it; the continuation decision is AIEN's)
//!   `discard`  `request_id`                     -> `{"ok":true,"discarded":bool}`
//! Anything else (unknown op, malformed JSON, wrong shape, bad trace id) is `{"ok":false,"error":..}`
//! and never a decision.
use std::collections::HashSet;
use std::io::{BufRead, Write};

use interplane_adapter_aien::{host_clock, AienAuthority};
use interplane_core::{CapabilityRequest, Decision, Exposure, Party};
use interplane_crossveil::{CallContext, RuntimeAuthority};
use serde_json::{json, Map, Value};

const DEFAULT_TTL_SECS: u64 = 300;
const MAX_TTL_SECS: u64 = 3600;

fn err(msg: impl Into<String>) -> Value {
    json!({"ok": false, "error": msg.into()})
}

fn field<T: serde::de::DeserializeOwned>(obj: &Map<String, Value>, key: &str) -> Result<T, String> {
    let v = obj
        .get(key)
        .ok_or_else(|| format!("missing field: {key}"))?;
    serde_json::from_value(v.clone()).map_err(|e| format!("bad {key}: {e}"))
}

fn is_trace_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn context(obj: &Map<String, Value>) -> Result<CallContext, String> {
    let ctx = obj
        .get("ctx")
        .and_then(Value::as_object)
        .ok_or("missing field: ctx")?;
    let text = |k: &str| -> Result<String, String> {
        ctx.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("ctx.{k} must be a string"))
    };
    let trace_id = text("trace_id")?;
    if !is_trace_id(&trace_id) {
        return Err("ctx.trace_id must be 32 lowercase hex characters".into());
    }
    let parent_id = match ctx.get("parent_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("ctx.parent_id must be a string or null".into()),
    };
    let model = match ctx.get("model") {
        None | Some(Value::Null) => Party::new("model", "unknown"),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("bad ctx.model: {e}"))?,
    };
    let exposure = match ctx.get("exposure") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value::<Exposure>(v.clone())
                .map_err(|e| format!("bad ctx.exposure: {e}"))?,
        ),
    };
    Ok(CallContext {
        trace_id,
        message_id: text("message_id")?,
        parent_id,
        model,
        session: ctx.get("runtime_session").filter(|v| !v.is_null()).cloned(),
        exposure,
    })
}

fn to_value<T: serde::Serialize>(key: &str, v: &T) -> Value {
    match serde_json::to_value(v) {
        Ok(x) => json!({"ok": true, key: x}),
        Err(e) => err(format!("cannot serialize {key}: {e}")),
    }
}

fn handle(aien: &mut AienAuthority, approved: &mut HashSet<String>, line: &str) -> Value {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return err(format!("malformed json: {e}")),
    };
    let Some(obj) = parsed.as_object() else {
        return err("request must be a json object");
    };
    let Some(op) = obj.get("op").and_then(Value::as_str) else {
        return err("missing field: op");
    };
    let mut run = || -> Result<Value, String> {
        match op {
            "catalog" => Ok(to_value("catalog", &aien.catalog())),
            "mapping_table" => Ok(to_value("mapping_table", &aien.mapping_table(true))),
            "decide" => {
                let req: CapabilityRequest = field(obj, "request")?;
                let ctx = context(obj)?;
                Ok(to_value("decision", &aien.decide(&req, &ctx)))
            }
            "execute" => {
                let req: CapabilityRequest = field(obj, "request")?;
                let decision: Decision = field(obj, "decision")?;
                let ctx = context(obj)?;
                Ok(to_value("result", &aien.execute(&req, &decision, &ctx)))
            }
            "approve" => {
                let req: CapabilityRequest = field(obj, "request")?;
                let ttl = match obj.get("ttl_secs") {
                    None | Some(Value::Null) => DEFAULT_TTL_SECS,
                    Some(v) => v
                        .as_u64()
                        .filter(|t| (1..=MAX_TTL_SECS).contains(t))
                        .ok_or("ttl_secs must be an integer from 1 to 3600")?,
                };
                // Restriction only: one approval call per request id. AIEN's desk would issue another grant.
                if approved.contains(&req.request_id) {
                    return Err(format!("approval already requested for {}", req.request_id));
                }
                let now = (host_clock())();
                let grant = aien.issue_approval(&req, now + ttl)?;
                let decision = aien.present_approval(&req, &grant, now);
                if decision.is_authorized() {
                    approved.insert(req.request_id.clone());
                }
                Ok(to_value("decision", &decision))
            }
            "discard" => {
                let rid: String = field(obj, "request_id")?;
                Ok(json!({"ok": true, "discarded": aien.discard_unexecuted(&rid)}))
            }
            other => Err(format!("unknown op: {other}")),
        }
    };
    run().unwrap_or_else(err)
}

fn main() {
    let Some(workspace) = std::env::args().nth(1) else {
        eprintln!("usage: aien-authority-bridge <workspace>");
        std::process::exit(2);
    };
    let mut aien = match AienAuthority::new(&workspace) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("aien-authority-bridge: {e}");
            std::process::exit(2);
        }
    };
    let mut approved = HashSet::new();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = handle(&mut aien, &mut approved, &line);
        if writeln!(out, "{answer}")
            .and_then(|()| out.flush())
            .is_err()
        {
            break;
        }
    }
}
