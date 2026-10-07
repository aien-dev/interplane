//! Who authored the proposal: the `model_turn` and `generation` records.
//!
//! The approved write path never calls the model, so the content of an effect only comes from the
//! loaded model if the model produced the tool-call text UPSTREAM, in the same trace. This module
//! checks that claim as far as the retained bytes allow and says exactly what stays unproven.
//!
//! `effect.proposal_origin`:
//! - `scripted_turn`: the INTERPLANE model turn was written by the harness. The link "this model
//!   produced this effect" is missing: `link:model_turn`.
//! - `model_generation/1`: `model_turn` (the exact text the dialect parsed) and `generation` (what
//!   the run driver saw AIEN's own generation path return) are retained. Checked here: the text
//!   re-parses, with the real `aien_legacy` dialect, into the trace's request; the generation
//!   output is that text by digest; the generation names the model digest the daemon load log
//!   carries (already bound to the export) and the same daemon process as the ledger.
//!   `generation.written_by` is `run-driver`: the daemon writes no record of a generation, so the
//!   link between generated text and model digest is asserted by the driver, not by the daemon:
//!   `link:daemon_generation_record` stays missing. `written_by: daemon` has no binding and is
//!   refused rather than half-checked.
//!
//! Reads and compares only. It never authorizes or executes anything.

use super::{fail, s, Archive, Call, Fail};
use crate::gojson::sha256_hex;
use interplane_lenshift::{DialectRegistry, ParseContext};
use serde_json::{json, Value};

pub const SCRIPTED: &str = "scripted_turn";
pub const GENERATED: &str = "model_generation/1";
pub const MISSING_MODEL_TURN: &str = "link:model_turn";
pub const MISSING_DAEMON_RECORD: &str = "link:daemon_generation_record";

/// Missing links implied by the effect's declared origin. A pure function of the manifest and the
/// retained `generation` record, so the completeness label can be compared before the deeper checks.
pub fn implied_missing(a: &Archive, m: &Value) -> Vec<String> {
    let Some(e) = m.get("effect").filter(|e| e.is_object()) else {
        return vec![];
    };
    match s(e, &["binding"]) {
        Some(crate::binding::LEDGER_BINDING) => match s(e, &["proposal_origin"]) {
            Some(SCRIPTED) => vec![MISSING_MODEL_TURN.into()],
            Some(GENERATED) => match a.json("generation") {
                Ok(Some(g)) if s(&g, &["written_by"]) == Some("run-driver") => {
                    vec![MISSING_DAEMON_RECORD.into()]
                }
                _ => vec![],
            },
            _ => vec![],
        },
        // The loose receipt names no request, trace or author: the model link is missing there too.
        Some(crate::binding::RECEIPT_BINDING) => vec![MISSING_MODEL_TURN.into()],
        _ => vec![],
    }
}

/// Checks for `proposal_origin = model_generation/1`.
pub fn check(a: &Archive, t: &Value, call: Option<&Call>) -> Result<(), Fail> {
    let Some(call) = call else {
        return fail(
            "missing_record",
            "interplane_trace (model_generation/1 needs it)",
        );
    };
    let (Some(turn), Some(gen)) = (a.json("model_turn")?, a.json("generation")?) else {
        return fail("missing_record", "model_turn/generation (not retained)");
    };
    match s(&gen, &["written_by"]) {
        Some("run-driver") => {}
        Some(other) => {
            return fail(
                "unsupported_binding",
                format!(
                    "generation.written_by={other} (no daemon generation record exists to bind)"
                ),
            )
        }
        None => return fail("malformed_companion", "generation.written_by"),
    }
    if s(&gen, &["kind"]) != Some("aien-generation-observation") {
        return fail("binding_mismatch", "generation.kind");
    }
    // 1. The text the dialect parsed is the text the generation returned.
    let (Some(text), Some(model), Some(dialect)) = (
        s(&turn, &["input"]),
        s(&turn, &["model"]),
        s(&turn, &["dialect"]),
    ) else {
        return fail("malformed_companion", "model_turn.input/model/dialect");
    };
    if s(&gen, &["output_text"]) != Some(text) {
        return fail(
            "binding_mismatch",
            "generation.output_text != model_turn.input",
        );
    }
    if s(&gen, &["output_sha256"]) != Some(sha256_hex(text.as_bytes()).as_str()) {
        return fail("binding_mismatch", "generation.output_sha256");
    }
    if dialect != "aien_legacy" {
        return fail(
            "unsupported_binding",
            format!("model_turn.dialect={dialect}"),
        );
    }
    // 2. The model that generated is the model the daemon loaded (load log digest, itself bound to
    //    the exported weights), in the same daemon process that wrote the ledger.
    let weights = a.sha("export_weights")?;
    if s(&gen, &["model_sha256_from_load_log"]) != Some(weights) {
        return fail(
            "binding_mismatch",
            "generation.model_sha256 != export_weights",
        );
    }
    let run = a.json("daemon_run")?.unwrap_or(Value::Null);
    if gen.get("daemon_pid") != run.get("pid")
        || gen.get("daemon_start_ticks") != run.get("start_ticks")
    {
        return fail(
            "binding_mismatch",
            "generation.daemon_pid/start vs daemon_run",
        );
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
    Ok(())
}
