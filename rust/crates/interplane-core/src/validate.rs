//! Structural envelope validation: what the schemas express. Never authorization.
use crate::types::{Envelope, ErrorCode, Party, Payload};
use crate::version::ProtocolVersion;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

const KINDS: [&str; 8] = [
    "tool_request",
    "capability_request",
    "decision",
    "result",
    "event",
    "catalog",
    "selection",
    "probe_report",
];
const PARTY_KINDS: [&str; 5] = ["model", "runtime", "adapter", "operator", "other"];

fn id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z0-9._:-]{1,128}$").expect("regex"))
}
fn ts_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$")
            .expect("regex")
    })
}

/// `common.schema.json#/$defs/Id`.
pub fn is_valid_id(s: &str) -> bool {
    id_re().is_match(s)
}

fn party_ok(p: &Party) -> bool {
    PARTY_KINDS.contains(&p.kind.as_str()) && !p.id.is_empty() && p.id.chars().count() <= 256
}

/// Validate a parsed envelope. Checks, in order: version (`unsupported_version` before anything
/// else is looked at), ids, timestamp, parties, payload kind and payload shape.
pub fn validate_envelope(e: &Envelope) -> Result<(), ErrorCode> {
    ProtocolVersion::parse(&e.interplane_version)?.check_major()?;
    if !is_valid_id(&e.message_id) || !is_valid_id(&e.trace_id) {
        return Err(ErrorCode::MalformedEnvelope);
    }
    if let Some(p) = &e.parent_id {
        if !is_valid_id(p) {
            return Err(ErrorCode::MalformedEnvelope);
        }
    }
    if !ts_re().is_match(&e.timestamp) || !party_ok(&e.source) || !party_ok(&e.destination) {
        return Err(ErrorCode::MalformedEnvelope);
    }
    let kind = e
        .payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(ErrorCode::MalformedEnvelope)?;
    if !KINDS.contains(&kind) {
        return Err(ErrorCode::MalformedEnvelope);
    }
    let payload = Payload::from_value(&e.payload)?;
    if let Payload::ToolRequest(t) = &payload {
        if !is_valid_id(&t.request_id) {
            return Err(ErrorCode::MalformedEnvelope);
        }
    }
    Ok(())
}

/// Validate raw JSON as an envelope. The version is checked first, even if the rest is malformed.
pub fn validate_envelope_value(v: &Value) -> Result<Envelope, ErrorCode> {
    let obj = v.as_object().ok_or(ErrorCode::MalformedEnvelope)?;
    match obj.get("interplane_version").and_then(Value::as_str) {
        Some(s) => ProtocolVersion::parse(s)?.check_major()?,
        None => return Err(ErrorCode::MalformedEnvelope),
    }
    let e: Envelope =
        serde_json::from_value(v.clone()).map_err(|_| ErrorCode::MalformedEnvelope)?;
    validate_envelope(&e)?;
    Ok(e)
}
