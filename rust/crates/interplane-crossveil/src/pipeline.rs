//! The reference pipeline: admit -> map -> decide -> execute -> render.
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use interplane_core::*;
use interplane_crossaxis::{coerce_arguments, MappingTable};
use interplane_lenshift::{DialectRegistry, ParseContext, RejectedCall};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{CallContext, RuntimeAuthority};

/// Deterministic per-request log entry (no timestamps, no durations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedRecord {
    pub request_id: Option<String>,
    pub stage: String,
    pub decision: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub decide_invoked: bool,
    pub execute_invoked: bool,
    pub result_digest: String,
}

/// Everything one model turn produced.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub turn: u64,
    pub intents: Vec<ToolRequest>,
    pub rejected: Vec<RejectedCall>,
    pub records: Vec<ObservedRecord>,
    pub results: Vec<ToolResult>,
    pub rendered: Vec<Value>,
    pub text: String,
    /// `tool_request`, `no_tool` or `rejected` (conformance README rule 4).
    pub outcome: String,
    /// Set when the whole turn was refused (`unsupported_dialect`).
    pub error_code: Option<ErrorCode>,
    pub partial: bool,
}

/// The pipeline. Holds no decision memory: every request is decided by the runtime.
pub struct Pipeline<'a> {
    pub registry: DialectRegistry,
    pub table: MappingTable,
    runtime: &'a mut dyn RuntimeAuthority,
    pub limits: Limits,
    pub ledger: RequestLedger,
    /// RelayLine events, in order.
    pub events: Vec<Event>,
    /// Prefix of generated message ids: `<prefix>-<turn>-<n>`.
    pub message_prefix: String,
    pub timestamp: String,
    seq: HashMap<String, u64>,
    turn: Option<u64>,
    /// Per trace: every input that has been placed in front of the model, in order (cut P3).
    inputs: HashMap<String, Vec<InputRecord>>,
}

/// Position in the trust order; `unknown` and anything unrecognized rank as `external_untrusted`.
fn trust_rank(t: &TrustLevel) -> u8 {
    match t {
        TrustLevel::TrustedRuntime => 3,
        TrustLevel::UserSupplied => 2,
        TrustLevel::WorkspaceUntrusted => 1,
        _ => 0,
    }
}

fn rank_trust(rank: u8) -> TrustLevel {
    match rank {
        3 => TrustLevel::TrustedRuntime,
        2 => TrustLevel::UserSupplied,
        1 => TrustLevel::WorkspaceUntrusted,
        _ => TrustLevel::ExternalUntrusted,
    }
}

fn rec(
    request_id: Option<&str>,
    stage: State,
    decision: Option<&str>,
    r: &ToolResult,
    decide: bool,
    exec: bool,
) -> ObservedRecord {
    ObservedRecord {
        request_id: request_id.map(str::to_string),
        stage: stage.name().to_string(),
        decision: decision.map(str::to_string),
        status: r.status.as_str().to_string(),
        error_code: r.error.as_ref().map(|e| e.code.as_str().to_string()),
        decide_invoked: decide,
        execute_invoked: exec,
        result_digest: r.result_digest(),
    }
}

fn valid_rid(v: &Value) -> Option<String> {
    v.pointer("/payload/request_id")
        .and_then(Value::as_str)
        .filter(|s| is_valid_id(s))
        .map(str::to_string)
}

/// Name the first field that makes an envelope malformed (for the pinned message).
fn malformed_field(v: &Value) -> String {
    const REQUIRED: [&str; 7] = [
        "interplane_version",
        "message_id",
        "trace_id",
        "timestamp",
        "source",
        "destination",
        "payload",
    ];
    for k in REQUIRED {
        if v.get(k).is_none() {
            return k.into();
        }
    }
    for k in ["message_id", "trace_id"] {
        if !v[k].as_str().is_some_and(is_valid_id) {
            return k.into();
        }
    }
    if let Some(p) = v.get("parent_id").filter(|p| !p.is_null()) {
        if !p.as_str().is_some_and(is_valid_id) {
            return "parent_id".into();
        }
    }
    for k in ["source", "destination"] {
        if serde_json::from_value::<Party>(v[k].clone()).is_err() {
            return k.into();
        }
    }
    if !v["timestamp"].is_string() {
        return "timestamp".into();
    }
    let Some(p) = v["payload"].as_object() else {
        return "payload".into();
    };
    if !p.get("kind").and_then(Value::as_str).is_some_and(|k| {
        [
            "tool_request",
            "capability_request",
            "decision",
            "result",
            "event",
            "catalog",
            "selection",
            "probe_report",
        ]
        .contains(&k)
    }) {
        return "payload.kind".into();
    }
    if p["kind"] == "tool_request" {
        for k in ["request_id", "tool", "arguments", "provenance"] {
            if !p.contains_key(k) {
                return format!("payload.{k}");
            }
        }
        if !v["payload"]["request_id"].as_str().is_some_and(is_valid_id) {
            return "payload.request_id".into();
        }
    }
    "payload".into()
}

impl<'a> Pipeline<'a> {
    pub fn new(
        registry: DialectRegistry,
        table: MappingTable,
        runtime: &'a mut dyn RuntimeAuthority,
        limits: Limits,
        ledger: RequestLedger,
    ) -> Self {
        Self {
            registry,
            table,
            runtime,
            limits,
            ledger,
            events: vec![],
            message_prefix: "m".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            seq: HashMap::new(),
            turn: None,
            inputs: HashMap::new(),
        }
    }

    /// Host-only: register an input (user request, workspace file, memory hit, skill, ...) placed
    /// in front of the model on `rec.trace_id`, before the turn that can see it. This is not
    /// reachable from model output or from an admitted envelope: those paths never call it.
    /// Refuses a duplicate `input_id` on the trace.
    pub fn register_input(&mut self, rec: InputRecord) -> Result<(), String> {
        let ledger = self.inputs.entry(rec.trace_id.clone()).or_default();
        if ledger.iter().any(|r| r.input_id == rec.input_id) {
            return Err(format!("duplicate input_id: {}", rec.input_id));
        }
        ledger.push(rec);
        Ok(())
    }

    /// The trace's input ledger, in registration order.
    pub fn inputs(&self, trace: &str) -> &[InputRecord] {
        self.inputs.get(trace).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Exposure computed from the ledger. Fails closed: an empty ledger means the host told us
    /// nothing about what the model saw, an input with unknown trust ranks as `external_untrusted`,
    /// and an input derived from an id the ledger does not hold counts as `external_untrusted`.
    pub fn exposure_for(&self, trace: &str) -> Exposure {
        let ledger = self.inputs(trace);
        let mut floor = if ledger.is_empty() { 0 } else { 3 };
        for r in ledger {
            let traced = r
                .derived_from
                .iter()
                .all(|d| ledger.iter().any(|o| &o.input_id == d));
            floor = floor.min(if traced { trust_rank(&r.trust) } else { 0 });
        }
        Exposure {
            inputs: ledger.iter().map(|r| r.input_id.clone()).collect(),
            floor: rank_trust(floor),
            extensions: Map::new(),
        }
    }

    /// Record a rendered result as an input of the trace (`parent_id` = its request id). The
    /// trust and kind are the runtime's normalized provenance; absent means `unknown`.
    fn record_result(&mut self, trace: &str, r: &ToolResult, rendered: &Value) {
        let prov = r.provenance.as_ref();
        let ledger = self.inputs.entry(trace.to_string()).or_default();
        let mut n = ledger
            .iter()
            .filter(|o| o.input_id.starts_with("in-auto-"))
            .count();
        while ledger.iter().any(|o| o.input_id == format!("in-auto-{n}")) {
            n += 1;
        }
        ledger.push(InputRecord {
            input_id: format!("in-auto-{n}"),
            content_kind: prov
                .and_then(|p| p.content_kind.clone())
                .unwrap_or(ContentKind::Undetermined),
            trust: prov
                .and_then(|p| p.trust.clone())
                .unwrap_or(TrustLevel::Undetermined),
            source: Party::new("runtime", self.runtime.runtime_id()),
            origin: format!("runtime:{}", self.runtime.runtime_id()),
            content_digest: digest(rendered),
            trace_id: trace.to_string(),
            parent_id: r.request_id.clone(),
            derived_from: vec![],
            extensions: Map::new(),
        });
    }

    fn emit(&mut self, trace: &str, event: EventKind, request_id: Option<&str>) {
        let s = self.seq.entry(trace.to_string()).or_insert(0);
        let seq = *s;
        *s += 1;
        self.events.push(Event {
            kind: EventMsgKind,
            event,
            seq,
            request_id: request_id.map(str::to_string),
            turn: self.turn,
            payload: None,
            extensions: Map::new(),
        });
    }

    fn emit_outcome(&mut self, trace: &str, r: &ToolResult) {
        let ev = match r.status {
            ResultStatus::Ok | ResultStatus::Denied | ResultStatus::RequiresApproval => {
                EventKind::ToolResult
            }
            _ => EventKind::ToolError,
        };
        let rid = r.request_id.clone();
        self.emit(trace, ev, rid.as_deref());
    }

    /// A refusal before the runtime was reached: provenance is all null.
    fn reject(
        &mut self,
        trace: &str,
        rid: Option<&str>,
        code: ErrorCode,
        msg: &str,
    ) -> (ToolResult, ObservedRecord) {
        let r = ToolResult::failed(rid, ResultStatus::Rejected, code, msg);
        let o = rec(rid, State::Rejected, None, &r, false, false);
        self.emit_outcome(trace, &r);
        (r, o)
    }

    /// Admit a raw JSON envelope: version, replay, duplicate, size, then map/decide/execute.
    pub fn admit_value(&mut self, v: &Value) -> (ToolResult, ObservedRecord) {
        let trace = v
            .get("trace_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let peek = valid_rid(v);
        // Version first, before anything else is read.
        match v
            .get("interplane_version")
            .and_then(Value::as_str)
            .map(ProtocolVersion::parse)
        {
            Some(Ok(ver)) => {
                if ver.check_major().is_err() {
                    let msg = format!("unsupported protocol major version: {}", ver.major);
                    return self.reject(&trace, None, ErrorCode::UnsupportedVersion, &msg);
                }
            }
            _ => {
                return self.reject(
                    &trace,
                    peek.as_deref(),
                    ErrorCode::MalformedEnvelope,
                    "malformed envelope: interplane_version",
                )
            }
        }
        let env: Envelope = match serde_json::from_value(v.clone()) {
            Ok(e) => e,
            Err(_) => {
                let msg = format!("malformed envelope: {}", malformed_field(v));
                return self.reject(&trace, peek.as_deref(), ErrorCode::MalformedEnvelope, &msg);
            }
        };
        self.admit_checked(&env, Some(v), None)
    }

    /// Admit a parsed envelope. Order: version, structure, replay, duplicate, size, map, decide, execute.
    pub fn admit_envelope(&mut self, env: &Envelope) -> (ToolResult, ObservedRecord) {
        self.admit_checked(env, None, None)
    }

    fn admit_checked(
        &mut self,
        env: &Envelope,
        raw: Option<&Value>,
        turn_exposure: Option<&Exposure>,
    ) -> (ToolResult, ObservedRecord) {
        let trace = env.trace_id.clone();
        let raw_v = match raw {
            Some(v) => v.clone(),
            None => serde_json::to_value(env).unwrap_or(Value::Null),
        };
        let peek = valid_rid(&raw_v);
        match ProtocolVersion::parse(&env.interplane_version) {
            Ok(v) if v.check_major().is_ok() => {}
            Ok(v) => {
                let msg = format!("unsupported protocol major version: {}", v.major);
                return self.reject(&trace, None, ErrorCode::UnsupportedVersion, &msg);
            }
            Err(_) => {
                return self.reject(
                    &trace,
                    peek.as_deref(),
                    ErrorCode::MalformedEnvelope,
                    "malformed envelope: interplane_version",
                )
            }
        }
        if let Err(code) = validate_envelope(env) {
            let msg = if code == ErrorCode::MalformedToolCall {
                "arguments is not an object".to_string()
            } else {
                format!("malformed envelope: {}", malformed_field(&raw_v))
            };
            return self.reject(&trace, peek.as_deref(), code, &msg);
        }
        if self.ledger.note_message(&trace, &env.message_id).is_err() {
            let msg = format!("replayed message_id: {}", env.message_id);
            return self.reject(&trace, peek.as_deref(), ErrorCode::ReplayedMessage, &msg);
        }
        let mut req = match env.parse_payload() {
            Ok(Payload::ToolRequest(t)) => t,
            _ => {
                return self.reject(
                    &trace,
                    peek.as_deref(),
                    ErrorCode::MalformedEnvelope,
                    "malformed envelope: payload.kind",
                )
            }
        };
        let rid = req.request_id.clone();
        self.emit(&trace, EventKind::ToolRequest, Some(&rid));
        if self.ledger.note_request(&trace, &rid).is_err() {
            let msg = format!("duplicate request_id: {rid}");
            return self.reject(&trace, Some(&rid), ErrorCode::DuplicateRequestId, &msg);
        }
        let size = canonicalize(&Value::Object(req.arguments.clone())).len();
        if size > self.limits.max_argument_bytes {
            let msg = format!("arguments exceed {} bytes", self.limits.max_argument_bytes);
            return self.reject(&trace, Some(&rid), ErrorCode::OversizedArguments, &msg);
        }
        // Exposure is the pipeline's own: whatever the model or envelope claimed is overwritten.
        let exposure = match turn_exposure {
            Some(e) => e.clone(),
            None => self.exposure_for(&trace),
        };
        req.provenance.extensions.insert(
            "exposure".into(),
            serde_json::to_value(&exposure).unwrap_or_else(
                |_| json!({"inputs": [], "floor": TrustLevel::ExternalUntrusted.as_str()}),
            ),
        );
        let ctx = CallContext {
            trace_id: trace.clone(),
            message_id: env.message_id.clone(),
            parent_id: env.parent_id.clone(),
            model: env.source.clone(),
            session: None,
            exposure: Some(exposure),
        };
        self.run_request(&req, &ctx)
    }

    fn run_request(
        &mut self,
        req: &ToolRequest,
        ctx: &CallContext,
    ) -> (ToolResult, ObservedRecord) {
        let trace = ctx.trace_id.clone();
        let rid = req.request_id.clone();
        let mut lc = Lifecycle::new(&rid);
        let mut cap = match self.table.map(req) {
            Ok(c) => c,
            Err(code) => {
                let _ = lc.reject();
                let name = match &req.tool.namespace {
                    Some(ns) => format!("{ns}.{}", req.tool.name),
                    None => req.tool.name.clone(),
                };
                return self.reject(
                    &trace,
                    Some(&rid),
                    code,
                    &format!("no mapping for tool: {name}"),
                );
            }
        };
        let _ = lc.map();
        let catalog = self.runtime.catalog();
        if let Some(pinned) = &cap.mapping.catalog_digest {
            let live = catalog
                .catalog_digest
                .clone()
                .unwrap_or_else(|| catalog.compute_digest());
            if *pinned != live {
                let _ = &lc;
                return self.reject(
                    &trace,
                    Some(&rid),
                    ErrorCode::StaleCapability,
                    "mapping table catalog digest does not match runtime catalog",
                );
            }
        }
        if let Some(d) = catalog
            .capabilities
            .iter()
            .find(|c| c.name == cap.capability)
        {
            coerce_arguments(&mut cap, d);
        }
        self.emit(&trace, EventKind::ToolMapped, Some(&rid));

        let runtime_name = self.runtime.runtime_id().to_string();
        // Provenance for results produced after the runtime was reached but without executing:
        // no data was produced, so `content_kind` and `trust` stay null (CORE.md).
        let reached = |cap: &CapabilityRequest| ResultProvenance {
            runtime: Some(runtime_name.clone()),
            capability: Some(cap.capability.clone()),
            content_kind: None,
            trust: None,
            ..Default::default()
        };

        // decide: fail closed on adapter panic.
        let decided = catch_unwind(AssertUnwindSafe(|| self.runtime.decide(&cap, ctx)));
        let decision = match decided {
            Ok(d) => d,
            Err(_) => {
                let mut r = ToolResult::failed(
                    Some(&rid),
                    ResultStatus::Denied,
                    ErrorCode::RuntimeUnavailable,
                    "runtime authority raised an error",
                );
                r.provenance = Some(reached(&cap));
                let o = rec(Some(&rid), State::Denied, Some("denied"), &r, true, false);
                self.emit_outcome(&trace, &r);
                return (r, o);
            }
        };
        self.emit(&trace, EventKind::ToolDecision, Some(&rid));
        let state = match lc.apply_decision(&decision) {
            Ok(s) => s,
            Err(_) => {
                // Cited a different request: nothing about it can be trusted. Denied.
                let mut r = ToolResult::failed(
                    Some(&rid),
                    ResultStatus::Denied,
                    ErrorCode::UnknownDecision,
                    "unknown decision value: mismatched request_id",
                );
                r.provenance = Some(reached(&cap));
                let o = rec(Some(&rid), State::Denied, Some("denied"), &r, true, false);
                self.emit_outcome(&trace, &r);
                return (r, o);
            }
        };
        let reason = decision.reason.clone();
        let non_exec = |status: ResultStatus, code: ErrorCode, msg: &str, embed: bool| {
            let mut r = ToolResult::failed(Some(&rid), status, code, msg);
            if embed {
                r.decision = Some(decision.clone());
            }
            r.provenance = Some(reached(&cap));
            r
        };
        let (result, dstr) = match state {
            State::Denied if !decision.decision.is_known() => (
                non_exec(
                    ResultStatus::Denied,
                    ErrorCode::UnknownDecision,
                    &format!("unknown decision value: {}", decision.decision.as_str()),
                    false,
                ),
                "denied".to_string(),
            ),
            State::Denied => (
                non_exec(
                    ResultStatus::Denied,
                    ErrorCode::PolicyDenied,
                    reason.as_deref().unwrap_or("denied by runtime policy"),
                    true,
                ),
                "denied".into(),
            ),
            State::RequiresApproval => {
                let id = decision
                    .approval
                    .as_ref()
                    .map(|a| a.approval_id.as_str())
                    .unwrap_or("");
                (
                    non_exec(
                        ResultStatus::RequiresApproval,
                        ErrorCode::ApprovalRequired,
                        &format!("approval required: {id}"),
                        true,
                    ),
                    "requires_approval".into(),
                )
            }
            State::Rejected if decision.decision == DecisionKind::NotFound => (
                non_exec(
                    ResultStatus::NotFound,
                    ErrorCode::CapabilityNotFound,
                    &format!("unknown capability: {}", cap.capability),
                    true,
                ),
                "not_found".into(),
            ),
            State::Rejected => (
                non_exec(
                    ResultStatus::Rejected,
                    ErrorCode::InvalidArguments,
                    reason.as_deref().unwrap_or("invalid arguments"),
                    true,
                ),
                "invalid".into(),
            ),
            State::Authorized => {
                let _ = lc.begin_execution(&decision);
                self.emit(&trace, EventKind::ToolExecutionStarted, Some(&rid));
                let ran = catch_unwind(AssertUnwindSafe(|| {
                    self.runtime.execute(&cap, &decision, ctx)
                }));
                let mut r = match ran {
                    Ok(r) => normalize(r, &cap),
                    Err(_) => {
                        let mut r = ToolResult::failed(
                            Some(&rid),
                            ResultStatus::Error,
                            ErrorCode::ExecutionError,
                            "runtime authority raised an error",
                        );
                        r.provenance = Some(reached(&cap));
                        r
                    }
                };
                let end = match r.status {
                    ResultStatus::Ok => State::Succeeded,
                    ResultStatus::TimedOut => State::TimedOut,
                    _ => State::Failed,
                };
                let _ = lc.finish(end);
                r.request_id = Some(rid.clone());
                let o = rec(Some(&rid), lc.state(), Some("authorized"), &r, true, true);
                self.emit_outcome(&trace, &r);
                return (r, o);
            }
            _ => unreachable!("apply_decision from MAPPED only yields the states above"),
        };
        let o = rec(Some(&rid), state, Some(&dstr), &result, true, false);
        self.emit_outcome(&trace, &result);
        (result, o)
    }

    /// Parse one model turn with a dialect, push every call through the pipeline, render results.
    pub fn run_turn(
        &mut self,
        dialect: &str,
        model: &str,
        input: &Value,
        trace_id: &str,
        turn: u64,
    ) -> TurnOutcome {
        self.turn = Some(turn);
        if turn > 0 {
            self.emit(trace_id, EventKind::Continue, None);
        }
        let mut out = TurnOutcome {
            turn,
            intents: vec![],
            rejected: vec![],
            records: vec![],
            results: vec![],
            rendered: vec![],
            text: String::new(),
            outcome: "rejected".into(),
            error_code: None,
            partial: false,
        };
        let parsed = match self.registry.get(dialect) {
            Ok(d) => d.parse(
                input,
                &ParseContext {
                    trace_id: trace_id.into(),
                    turn,
                    model: model.into(),
                },
            ),
            Err(code) => {
                out.error_code = Some(code);
                self.emit(trace_id, EventKind::ToolError, None);
                return out;
            }
        };
        let mut parsed = parsed;
        let exposure = self.exposure_for(trace_id);
        for intent in &mut parsed.intents {
            intent.provenance.extensions.insert(
                "exposure".into(),
                serde_json::to_value(&exposure).unwrap_or(Value::Null),
            );
        }
        out.text = parsed.text.clone();
        out.partial = parsed.partial;
        out.outcome = if !parsed.intents.is_empty() {
            "tool_request"
        } else if !parsed.rejected.is_empty() {
            "rejected"
        } else {
            "no_tool"
        }
        .into();
        if parsed.reasoning_digest.is_some() {
            self.emit(trace_id, EventKind::ReasoningDelta, None);
        }
        if !parsed.text.is_empty() {
            self.emit(trace_id, EventKind::ModelDelta, None);
        }

        // Interleave intents and rejected calls in emission order.
        enum Item {
            Intent(usize),
            Rejected(usize),
        }
        let mut items: Vec<(usize, Item)> = parsed
            .intent_indices
            .iter()
            .enumerate()
            .map(|(i, ix)| (*ix, Item::Intent(i)))
            .collect();
        items.extend(
            parsed
                .rejected
                .iter()
                .enumerate()
                .map(|(i, r)| (r.index, Item::Rejected(i))),
        );
        items.sort_by_key(|(ix, _)| *ix);

        let mut n = 0usize;
        for (_, item) in items {
            match item {
                Item::Rejected(i) => {
                    let r = &parsed.rejected[i];
                    let _ = self.ledger.note_request(trace_id, &r.request_id);
                    let (res, o) =
                        self.reject(trace_id, Some(&r.request_id), r.code.clone(), &r.message);
                    out.rendered.push(self.render(dialect, &res, None));
                    out.results.push(res);
                    out.records.push(o);
                }
                Item::Intent(i) => {
                    let intent = &parsed.intents[i];
                    let (res, o) = if n >= self.limits.max_requests_per_turn {
                        let _ = self.ledger.note_request(trace_id, &intent.request_id);
                        self.reject(
                            trace_id,
                            Some(&intent.request_id),
                            ErrorCode::MalformedToolCall,
                            "too many tool calls in one turn",
                        )
                    } else {
                        let env = Envelope::new(
                            &format!("{}-{}-{}", self.message_prefix, turn, n),
                            trace_id,
                            &self.timestamp.clone(),
                            Party::new("model", model),
                            Party::new("runtime", self.runtime.runtime_id()),
                            serde_json::to_value(intent).unwrap_or(Value::Null),
                        );
                        self.admit_checked(&env, None, Some(&exposure))
                    };
                    n += 1;
                    out.rendered.push(self.render(dialect, &res, Some(intent)));
                    out.results.push(res);
                    out.records.push(o);
                }
            }
        }
        // Everything rendered back to the model is visible to its next turn.
        for (res, rendered) in out.results.clone().iter().zip(&out.rendered) {
            self.record_result(trace_id, res, rendered);
        }
        if out.intents.is_empty() && parsed.intents.is_empty() && parsed.rejected.is_empty() {
            self.emit(trace_id, EventKind::Complete, None);
        }
        out.intents = parsed.intents;
        out.rejected = parsed.rejected;
        out
    }

    fn render(&self, dialect: &str, r: &ToolResult, intent: Option<&ToolRequest>) -> Value {
        self.registry
            .get(dialect)
            .map(|d| d.render_result(r, intent))
            .unwrap_or(Value::Null)
    }
}

/// Hold the runtime's execute result to the contract: executed statuses only, request id ours,
/// `content_kind`/`trust` defaulting to `tool_result`/`unknown`, `trusted` derived from `trust`.
fn normalize(mut r: ToolResult, cap: &CapabilityRequest) -> ToolResult {
    if !matches!(
        r.status,
        ResultStatus::Ok | ResultStatus::Error | ResultStatus::TimedOut
    ) {
        r = ToolResult::failed(
            Some(&cap.request_id),
            ResultStatus::Error,
            ErrorCode::ExecutionError,
            "runtime returned a non-execution status from execute",
        );
    }
    if r.status == ResultStatus::Ok {
        r.error = None;
    } else {
        r.data = Value::Null;
        r.decision = None;
        let e = r
            .error
            .get_or_insert_with(|| ErrorObject::new(ErrorCode::ExecutionError, "execution failed"));
        e.retryable = matches!(
            e.code,
            ErrorCode::ExecutionTimeout | ErrorCode::RuntimeUnavailable
        );
    }
    let p = r.provenance.get_or_insert_with(ResultProvenance::default);
    p.runtime.get_or_insert_with(|| cap.runtime.clone());
    p.capability.get_or_insert_with(|| cap.capability.clone());
    p.content_kind = Some(match p.content_kind.take() {
        None => ContentKind::ToolResult,
        Some(ContentKind::Unknown(_)) => ContentKind::Undetermined,
        Some(k) => k,
    });
    let trust = match p.trust.take() {
        None => TrustLevel::Undetermined,
        Some(TrustLevel::Unknown(_)) => TrustLevel::ExternalUntrusted,
        Some(t) => t,
    };
    p.trusted = trust.trusted_flag();
    p.trust = Some(trust);
    r
}
