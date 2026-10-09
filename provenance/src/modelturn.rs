//! Who authored the proposal: the `model_turn` and `generation` records.
//!
//! The approved write path never calls the model, so the content of an effect only comes from the
//! loaded model if the model produced the tool-call text UPSTREAM, in the same trace. This module
//! checks that claim as far as the retained bytes allow and says exactly what stays unproven.
//!
//! `effect.proposal_origin`:
//! - `scripted_turn`: the INTERPLANE model turn was written by the harness. The link "this model
//!   produced this effect" is missing: `link:model_turn`.
//! - `model_generation/2`: `model_turn` (the exact text the dialect parsed, plus the id the daemon
//!   returned in `TurnFinished.generation_record`) and `ledger_generation` (the DAEMON'S OWN record
//!   of that generation, exported with `ComposeRecall`, see sovereign-core
//!   docs/DAEMON_GENERATION_RECORD.md) are retained. Checked: the record is a verified `effect`
//!   note whose text is one JSON object with `generation == 1`, `v == 1` and no links; its
//!   model and tokenizer digests are the exported files; its output digest is the digest of the
//!   model turn text; the text re-parses, with the real `aien_legacy` dialect, into the trace's
//!   request; its daemon pid/start are the daemon that served the effect; it precedes the effect's
//!   replay claim in the ledger. Nothing is missing then, but see "Does not prove" in the doc:
//!   the hash-then-load gap, no signature, and a same-user process could append a record.
//! - `model_generation/1` (a record written by the run driver) is superseded and refused.
//!
//! Reads and compares only. It never authorizes or executes anything.

use super::{fail, s, Archive, Call, Fail};
use crate::gojson::sha256_hex;
use interplane_lenshift::{DialectRegistry, ParseContext};
use serde_json::{json, Value};

pub const SCRIPTED: &str = "scripted_turn";
pub const GENERATED: &str = "model_generation/2";
pub const SUPERSEDED: &str = "model_generation/1";
pub const MISSING_MODEL_TURN: &str = "link:model_turn";

/// Missing links implied by the effect's declared origin. A pure function of the manifest and the
/// retained `generation` record, so the completeness label can be compared before the deeper checks.
pub fn implied_missing(_a: &Archive, m: &Value) -> Vec<String> {
    let Some(e) = m.get("effect").filter(|e| e.is_object()) else {
        return vec![];
    };
    match s(e, &["binding"]) {
        Some(crate::binding::LEDGER_BINDING) => match s(e, &["proposal_origin"]) {
            Some(SCRIPTED) => vec![MISSING_MODEL_TURN.into()],
            Some(GENERATED) => vec![],
            _ => vec![],
        },
        // The loose receipt names no request, trace or author: the model link is missing there too.
        Some(crate::binding::RECEIPT_BINDING) => vec![MISSING_MODEL_TURN.into()],
        _ => vec![],
    }
}

/// Checks for `proposal_origin = model_generation/2`.
pub fn check(a: &Archive, t: &Value, call: Option<&Call>) -> Result<(), Fail> {
    let Some(call) = call else {
        return fail(
            "missing_record",
            "interplane_trace (model_generation/2 needs it)",
        );
    };
    let Some(turn) = a.json("model_turn")? else {
        return fail("missing_record", "model_turn (not retained)");
    };
    if !a.has("ledger_generation") {
        return fail("missing_record_entry", "ledger_generation");
    }
    let (Some(text), Some(model), Some(dialect)) = (
        s(&turn, &["input"]),
        s(&turn, &["model"]),
        s(&turn, &["dialect"]),
    ) else {
        return fail("malformed_companion", "model_turn.input/model/dialect");
    };
    if dialect != "aien_legacy" {
        return fail(
            "unsupported_binding",
            format!("model_turn.dialect={dialect}"),
        );
    }
    // 1. The daemon's own record of the generation.
    let Some(rec) = a.json("ledger_generation")? else {
        return fail("missing_record", "ledger_generation (not retained)");
    };
    let bad = |what: &str| -> Result<(), Fail> {
        fail("binding_mismatch", format!("ledger_generation.{what}"))
    };
    let Some(id) = rec.get("id").and_then(Value::as_u64) else {
        return bad("id");
    };
    if rec.get("verified") != Some(&Value::Bool(true)) {
        return bad("verified");
    }
    if s(&rec, &["note"]) != Some("effect") {
        return bad("note");
    }
    // "Empty links": the view carries four fixed slots, all zero.
    if !matches!(rec.get("links").and_then(Value::as_array),
        Some(l) if l.iter().all(|x| x.as_u64() == Some(0)))
    {
        return bad("links (must be empty)");
    }
    // Strict parse: the note text is exactly one JSON object.
    let Some(g) = rec
        .get("text")
        .and_then(Value::as_str)
        .and_then(|t| crate::strict::parse(t.as_bytes()))
        .filter(Value::is_object)
    else {
        return bad("text (not one JSON object)");
    };
    if g.get("generation").and_then(Value::as_u64) != Some(1)
        || g.get("v").and_then(Value::as_u64) != Some(1)
    {
        return bad("generation/v marker");
    }
    // The id the daemon returned to the client is the record that is retained.
    if turn.get("generation_record_id").and_then(Value::as_u64) != Some(id) {
        return fail(
            "binding_mismatch",
            "model_turn.generation_record_id != ledger_generation.id",
        );
    }
    // The digest the record carries is the digest of the very text the dialect parsed.
    if s(&g, &["output_text_sha256"]) != Some(sha256_hex(text.as_bytes()).as_str()) {
        return fail(
            "binding_mismatch",
            "generation record output_text_sha256 != sha256(model_turn.input)",
        );
    }
    // The model and tokenizer the daemon says it held are the exported files.
    if s(&g, &["model_sha256"]) != Some(a.sha("export_weights")?) {
        return fail(
            "binding_mismatch",
            "generation record model_sha256 != export_weights",
        );
    }
    if s(&g, &["tokenizer_sha256"]) != Some(a.sha("export_tokenizer")?) {
        return fail(
            "binding_mismatch",
            "generation record tokenizer_sha256 != export_tokenizer",
        );
    }
    match s(&g, &["finish_reason"]) {
        Some("eos") | Some("max_tokens") => {}
        other => {
            return fail(
                "binding_mismatch",
                format!(
                    "generation record finish_reason={}",
                    other.unwrap_or("absent")
                ),
            )
        }
    }
    if g.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) == 0 {
        return fail("binding_mismatch", "generation record output_tokens");
    }
    for k in ["prompt_ids_sha256", "output_token_ids_sha256"] {
        match s(&g, &[k]) {
            Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            _ => return fail("binding_mismatch", format!("generation record {k}")),
        }
    }
    // The daemon that wrote it is the daemon that served the effect.
    let run = a.json("daemon_run")?.unwrap_or(Value::Null);
    let d = g.get("daemon").unwrap_or(&Value::Null);
    if d.get("pid") != run.get("pid") || d.get("start_ticks") != run.get("start_ticks") {
        return fail(
            "binding_mismatch",
            "generation record daemon pid/start vs daemon_run",
        );
    }
    // The generation precedes the effect: before the replay claim of the approved write.
    let claim = a.json("ledger_claim")?.unwrap_or(Value::Null);
    match claim.get("id").and_then(Value::as_u64) {
        Some(c) if id < c => {}
        _ => {
            return fail(
                "binding_mismatch",
                "generation record does not precede the effect's replay claim",
            )
        }
    }
    // 3. The real dialect turns that text into this trace's request.
    let (Some(tid), Some(rid)) = (s(t, &["trace_id"]), s(t, &["request_id"])) else {
        return fail("malformed_companion", "interplane.trace_id/request_id");
    };
    if s(&turn, &["trace_id"]) != Some(tid) {
        return fail("binding_mismatch", "model_turn.trace_id");
    }
    let registry = DialectRegistry::with_defaults();
    let Ok(d) = registry.get("aien_legacy") else {
        return fail("unsupported_binding", "aien_legacy dialect unavailable");
    };
    let parsed = d.parse(
        &json!(text),
        &ParseContext {
            trace_id: tid.into(),
            turn: turn.get("turn").and_then(Value::as_u64).unwrap_or(0),
            model: model.into(),
        },
    );
    if parsed.intents.len() != 1 || !parsed.rejected.is_empty() || parsed.partial {
        return fail(
            "binding_mismatch",
            format!(
                "model_turn parses to {} request(s), {} rejected",
                parsed.intents.len(),
                parsed.rejected.len()
            ),
        );
    }
    let i = &parsed.intents[0];
    if i.request_id != rid {
        return fail(
            "binding_mismatch",
            format!("model_turn request_id {} vs interplane {rid}", i.request_id),
        );
    }
    if i.tool.name != call.tool {
        return fail("binding_mismatch", "model_turn tool vs trace request");
    }
    if serde_json::to_value(&i.arguments).ok().as_ref() != Some(&call.args) {
        return fail("binding_mismatch", "model_turn arguments vs trace request");
    }
    if let Some(sd) = call.req_payload["provenance"]["source_digest"].as_str() {
        if i.provenance.source_digest.as_deref() != Some(sd) {
            return fail(
                "binding_mismatch",
                "model_turn source_digest vs trace request",
            );
        }
    }
    // 4. The trace says the model sent it.
    if s(&call.req_source, &["kind"]) != Some("model")
        || s(&call.req_source, &["id"]) != Some(model)
    {
        return fail(
            "binding_mismatch",
            "trace request source vs model_turn.model",
        );
    }
    check_turn_record(a)?;
    Ok(())
}

fn is_rfc3339_utc(x: &str) -> bool {
    let b = x.as_bytes();
    b.len() == 20
        && b[19] == b'Z'
        && [4, 7].iter().all(|&i| b[i] == b'-')
        && b[10] == b'T'
        && [13, 16].iter().all(|&i| b[i] == b':')
        && b.iter()
            .enumerate()
            .all(|(i, c)| [4, 7, 10, 13, 16, 19].contains(&i) || c.is_ascii_digit())
}

/// `model-turn/2`: the retained model turn names the exact request and response bytes, the model
/// and the weights digest, the backend and the times, and each one is compared with what the
/// bundle retains. Runs only for a turn that declares `record: "model-turn/2"`; a turn without
/// the marker keeps the older, weaker checks above and is not read as carrying these links.
/// Compared, never trusted: the request bytes are the harness's own (the daemon's record holds
/// only the token-id digest of the prompt, which this offline check cannot recompute), and the
/// backend line is the daemon's log line quoted by the harness.
fn check_turn_record(a: &Archive) -> Result<(), Fail> {
    let Some(turn) = a.json("model_turn")? else {
        return fail("missing_record", "model_turn (not retained)");
    };
    if s(&turn, &["record"]) != Some("model-turn/2") {
        return Ok(());
    }
    let bad =
        |what: &str| -> Result<(), Fail> { fail("binding_mismatch", format!("model_turn.{what}")) };
    let hex64 = |x: Option<&str>| {
        x.is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    // Request bytes: the retained record is exactly the bytes whose digest the turn names.
    if s(&turn, &["request_record"]) != Some("model_request") {
        return bad("request_record");
    }
    if !a.has("model_request") {
        return fail("missing_record_entry", "model_request");
    }
    let Some(req) = a.bytes("model_request")? else {
        return fail("missing_record", "model_request (not retained)");
    };
    let req_sha = sha256_hex(req);
    if !hex64(s(&turn, &["request_sha256"])) || s(&turn, &["request_sha256"]) != Some(&req_sha) {
        return bad("request_sha256 != sha256(model_request)");
    }
    if a.sha("model_request")? != req_sha {
        return bad("request_sha256 != record digest");
    }
    // The request is a messages array that really asks for something (not an empty shell).
    match crate::strict::parse(req) {
        Some(Value::Array(m)) if !m.is_empty() => {}
        _ => return bad("model_request (not a non-empty messages array)"),
    }
    // Response bytes: the exact text the dialect parsed.
    let Some(text) = s(&turn, &["input"]) else {
        return bad("input");
    };
    if s(&turn, &["response_sha256"]) != Some(sha256_hex(text.as_bytes()).as_str()) {
        return bad("response_sha256 != sha256(input)");
    }
    // Weights: the digest the turn names is the exported file the daemon's record also names.
    if s(&turn, &["weights_sha256"]) != Some(a.sha("export_weights")?) {
        return bad("weights_sha256 != export_weights");
    }
    for k in ["model_id", "backend"] {
        if s(&turn, &[k]).is_none_or(str::is_empty) {
            return bad(k);
        }
    }
    // The backend line is quoted from the load log the bundle retains.
    let backend = s(&turn, &["backend"]).unwrap_or("");
    let log = a
        .bytes("aien_load_log")?
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    if !log.lines().any(|l| l.trim() == backend) {
        return bad("backend (not a line of aien_load_log)");
    }
    // Times: well formed, ordered, and the model finished before the request was admitted.
    let (Some(t0), Some(t1)) = (
        s(&turn, &["stream_started"]),
        s(&turn, &["stream_finished"]),
    ) else {
        return bad("stream_started/stream_finished");
    };
    if !is_rfc3339_utc(t0) || !is_rfc3339_utc(t1) || t0 > t1 {
        return bad("stream_started/stream_finished (order or form)");
    }
    let trace = a.json("interplane_trace")?.unwrap_or(Value::Null);
    match trace.get(0).and_then(|e| s(e, &["timestamp"])) {
        Some(req_t) if t1 <= req_t => {}
        _ => return bad("stream_finished is after the trace request"),
    }
    Ok(())
}
