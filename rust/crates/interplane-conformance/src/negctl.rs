//! Negative controls V1 to V6 (bench/PROTOCOL-0.3.md section 6): deliberately vulnerable variants
//! of the harness, so the suite can show it is able to fail. Compiled only with the test-only
//! `negative-controls` feature of this crate. No pipeline library knows about any of this: each
//! variant wraps the unmodified pipeline, runtime or lifecycle from the outside.
use std::collections::HashMap;
use std::path::Path;

use interplane_core::{CapabilityRequest, Decision, DecisionKind, RequestLedger};
use interplane_crossveil::{ObservedRecord, Pipeline};
use serde_json::{json, Value};

use crate::{run_suite, EFFECT_CAPABILITIES};

/// Printed by the runner when a variant is selected. CI greps the default build for it.
pub const MARKER: &str = "NEGCTL_BUILT_IN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
}

impl Variant {
    pub const ALL: [Variant; 6] = [
        Variant::V1,
        Variant::V2,
        Variant::V3,
        Variant::V4,
        Variant::V5,
        Variant::V6,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Variant::V1 => "V1",
            Variant::V2 => "V2",
            Variant::V3 => "V3",
            Variant::V4 => "V4",
            Variant::V5 => "V5",
            Variant::V6 => "V6",
        }
    }
    pub fn parse(s: &str) -> Option<Variant> {
        Variant::ALL.into_iter().find(|v| v.name() == s)
    }
}

fn is(ctl: Option<Variant>, v: Variant) -> bool {
    ctl == Some(v)
}

/// An approval asserted by the model: in `arguments` or in `extensions`.
fn claims_approval(req: &CapabilityRequest) -> bool {
    let claim = |m: &serde_json::Map<String, Value>| {
        m.get("approval_id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            || m.get("approved") == Some(&json!(true))
    };
    claim(&req.arguments) || claim(&req.extensions)
}

/// V2 and V4: the runtime's refusal is overturned. V2 honours a model-asserted approval; V4
/// ignores exposure and authorizes every effect capability.
pub fn tamper_decision(ctl: Option<Variant>, req: &CapabilityRequest, d: &mut Decision) {
    if !matches!(
        d.decision,
        DecisionKind::Denied | DecisionKind::RequiresApproval
    ) {
        return;
    }
    let fire = (is(ctl, Variant::V2) && claims_approval(req))
        || (is(ctl, Variant::V4) && EFFECT_CAPABILITIES.contains(&req.capability.as_str()));
    if fire {
        d.decision = DecisionKind::Authorized;
        d.reason = None;
        d.approval = None;
    }
}

/// V6: re-admission with duplicate detection off. Every step starts with a fresh ledger.
pub fn before_step(ctl: Option<Variant>, p: &mut Pipeline<'_>) {
    if is(ctl, Variant::V6) {
        p.ledger = RequestLedger::new();
    }
}

fn collect_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

/// V1: the host loop parses tool-call markup found in rendered results and inputs as if the model
/// had written it, and runs the resulting intents.
pub fn host_loop(
    ctl: Option<Variant>,
    p: &mut Pipeline<'_>,
    trace: &str,
    new: &[Value],
    observed: &mut Vec<ObservedRecord>,
    results: &mut Vec<Value>,
) {
    if !is(ctl, Variant::V1) {
        return;
    }
    let mut texts = vec![];
    new.iter().for_each(|r| collect_strings(r, &mut texts));
    for t in texts.into_iter().filter(|t| t.contains("<tool_call>")) {
        let turn = 1000 + observed.len() as u64;
        let out = p.run_turn("qwen35", "Qwen/Qwen3.5-9B", &Value::String(t), trace, turn);
        results.extend(
            out.results
                .iter()
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null)),
        );
        observed.extend(out.records);
    }
}

/// V3: absent or unknown trust defaults to `trusted_runtime` (results and the input ledger).
pub fn finish(ctl: Option<Variant>, results: &mut [Value], inputs: &mut [Value]) {
    if !is(ctl, Variant::V3) {
        return;
    }
    let weak = |t: Option<&Value>| t.is_none_or(|t| t.is_null() || t == "unknown");
    for r in results.iter_mut() {
        if let Some(prov) = r.get_mut("provenance").and_then(Value::as_object_mut) {
            if weak(prov.get("trust")) {
                prov.insert("trust".into(), json!("trusted_runtime"));
                prov.insert("trusted".into(), json!(true));
            }
        }
    }
    for rec in inputs.iter_mut() {
        if weak(rec.get("trust")) {
            rec["trust"] = json!("trusted_runtime");
        }
    }
}

/// V5: a continuation is accepted when it cites any non-empty `approval_id` (Rust lifecycle at
/// 678c06c). Modelled by presenting the minted id in place of whatever the caller cited.
pub fn continuation_any_id(
    ctl: Option<Variant>,
    minted: &mut HashMap<String, String>,
    rid: &str,
    d: &mut Decision,
) {
    if !is(ctl, Variant::V5) {
        return;
    }
    match (minted.get(rid), d.approval.as_mut()) {
        (None, Some(a)) => {
            minted.insert(rid.to_string(), a.approval_id.clone());
        }
        (Some(m), Some(a)) if !a.approval_id.is_empty() => a.approval_id = m.clone(),
        _ => {}
    }
}

/// Run the whole suite unmodified and once per variant; check each variant fails every case the
/// spec lists for it. Names the failing cases; `valid` is true only if the unmodified run is clean
/// and every variant is detected.
pub fn matrix(dir: &Path, spec: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(spec).map_err(|e| format!("{}: {e}", spec.display()))?;
    let spec: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let names = |s: &crate::Suite| -> Vec<String> {
        let mut v: Vec<String> = s.failed().iter().map(|o| o.name.clone()).collect();
        v.sort();
        v
    };
    let base = names(&run_suite(dir, None)?);
    let mut variants = serde_json::Map::new();
    let mut valid = base.is_empty();
    for v in Variant::ALL {
        let failed = names(&run_suite(dir, Some(v))?);
        let row = &spec["variants"][v.name()];
        let must: Vec<String> = row["must_fail"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let detected = !must.is_empty() && must.iter().all(|m| failed.contains(m));
        valid &= detected;
        variants.insert(
            v.name().into(),
            json!({"defect": row["defect"], "failed": failed, "must_fail": must, "detected": detected}),
        );
    }
    Ok(json!({"baseline_failed": base, "valid": valid, "variants": variants}))
}
