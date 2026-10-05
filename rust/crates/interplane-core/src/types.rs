//! Serde types for every INTERPLANE 0.1 schema. Unknown fields survive round trips
//! in the flattened `extensions` map.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::jcs::{canonicalize, digest};

string_enum! {
    /// `common.schema.json#/$defs/ErrorCode`. Unknown strings are kept (non-authority enum).
    ErrorCode {
        UnsupportedVersion => "unsupported_version",
        UnsupportedDialect => "unsupported_dialect",
        MalformedEnvelope => "malformed_envelope",
        MalformedToolCall => "malformed_tool_call",
        InvalidArguments => "invalid_arguments",
        OversizedArguments => "oversized_arguments",
        DuplicateRequestId => "duplicate_request_id",
        ReplayedMessage => "replayed_message",
        UnknownCapability => "unknown_capability",
        CapabilityNotFound => "capability_not_found",
        PolicyDenied => "policy_denied",
        ApprovalRequired => "approval_required",
        ExecutionError => "execution_error",
        ExecutionTimeout => "execution_timeout",
        UnknownDecision => "unknown_decision",
        RuntimeUnavailable => "runtime_unavailable",
        StaleCapability => "stale_capability",
    }
}

string_enum! {
    /// The runtime's decision value. An unknown value MUST be treated as denied.
    DecisionKind {
        Authorized => "authorized",
        Denied => "denied",
        RequiresApproval => "requires_approval",
        NotFound => "not_found",
        Invalid => "invalid",
    }
}

string_enum! {
    /// `result.status`.
    ResultStatus {
        Ok => "ok",
        Error => "error",
        TimedOut => "timed_out",
        Denied => "denied",
        RequiresApproval => "requires_approval",
        Rejected => "rejected",
        NotFound => "not_found",
    }
}

string_enum! {
    /// How far the runtime trusts material. Unknown strings are treated as `external_untrusted`.
    TrustLevel {
        TrustedRuntime => "trusted_runtime",
        UserSupplied => "user_supplied",
        WorkspaceUntrusted => "workspace_untrusted",
        ExternalUntrusted => "external_untrusted",
        Undetermined => "unknown",
    }
}

string_enum! {
    /// What a piece of model-visible material is. Unknown strings are treated as `unknown`.
    ContentKind {
        RuntimeInstruction => "runtime_instruction",
        UserRequest => "user_request",
        ModelGenerated => "model_generated",
        ToolResult => "tool_result",
        WorkspaceContent => "workspace_content",
        Memory => "memory",
        WebContent => "web_content",
        Document => "document",
        Email => "email",
        Skill => "skill",
        ExternalProvider => "external_provider",
        Undetermined => "unknown",
    }
}

string_enum! {
    /// RelayLine event names.
    EventKind {
        ModelDelta => "model_delta",
        ReasoningDelta => "reasoning_delta",
        ToolRequest => "tool_request",
        ToolMapped => "tool_mapped",
        ToolDecision => "tool_decision",
        ToolExecutionStarted => "tool_execution_started",
        ToolResult => "tool_result",
        ToolError => "tool_error",
        Continue => "continue",
        Complete => "complete",
        Cancelled => "cancelled",
    }
}

string_enum! {
    /// Probe verdicts.
    Verdict {
        Pass => "PASS",
        Fail => "FAIL",
        Degraded => "DEGRADED",
        Unsupported => "UNSUPPORTED",
        Undetermined => "UNKNOWN",
        Skipped => "SKIPPED",
    }
}

string_enum! {
    /// Profile status in a probe report.
    ProfileStatus {
        Compatible => "compatible",
        Incompatible => "incompatible",
        Untested => "untested",
    }
}

const_kind!(ToolRequestKind, "tool_request");
const_kind!(CapabilityRequestKind, "capability_request");
const_kind!(DecisionMsgKind, "decision");
const_kind!(ResultKind, "result");
const_kind!(EventMsgKind, "event");
const_kind!(CatalogKind, "catalog");
const_kind!(SelectionKind, "selection");
const_kind!(ProbeReportKind, "probe_report");

/// Free-form unknown fields, preserved verbatim.
pub type Extensions = Map<String, Value>;

fn is_none<T>(o: &Option<T>) -> bool {
    o.is_none()
}
fn is_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// A message participant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Party {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub trust: Option<TrustLevel>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub content_kind: Option<ContentKind>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Party {
    pub fn new(kind: &str, id: &str) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            trust: None,
            content_kind: None,
            extensions: Map::new(),
        }
    }
}

/// Canonical tool coordinates. `additionalProperties: false` in the schema, so no extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRef {
    #[serde(default)]
    pub namespace: Option<String>,
    pub name: String,
}

/// The transport-neutral wrapper (Vectorveil). `payload` is kept as JSON so an unknown or
/// malformed payload can be rejected without losing it; see [`Envelope::parse_payload`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub interplane_version: String,
    pub message_id: String,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub source: Party,
    pub destination: Party,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub signature: Option<Value>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Envelope {
    /// Wrap a payload with the current protocol version.
    pub fn new(
        message_id: &str,
        trace_id: &str,
        timestamp: &str,
        source: Party,
        destination: Party,
        payload: Value,
    ) -> Self {
        Self {
            interplane_version: crate::PROTOCOL_VERSION.into(),
            message_id: message_id.into(),
            trace_id: trace_id.into(),
            parent_id: None,
            timestamp: timestamp.into(),
            source,
            destination,
            payload,
            digest: None,
            signature: None,
            extensions: Map::new(),
        }
    }
    /// Parse the payload by its `kind`.
    pub fn parse_payload(&self) -> Result<Payload, ErrorCode> {
        Payload::from_value(&self.payload)
    }
}

/// A typed payload.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    ToolRequest(ToolRequest),
    CapabilityRequest(CapabilityRequest),
    Decision(Decision),
    Result(ToolResult),
    Event(Event),
    Catalog(Catalog),
    Selection(Selection),
    ProbeReport(ProbeReport),
}

impl Payload {
    /// Dispatch on `kind`. Unknown kind -> `malformed_envelope`; a `tool_request` whose
    /// `arguments` is present but not an object -> `malformed_tool_call`; other shape errors
    /// -> `malformed_envelope`.
    pub fn from_value(v: &Value) -> Result<Payload, ErrorCode> {
        let kind = v
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(ErrorCode::MalformedEnvelope)?;
        fn de<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ErrorCode> {
            serde_json::from_value(v.clone()).map_err(|_| ErrorCode::MalformedEnvelope)
        }
        Ok(match kind {
            "tool_request" => {
                if matches!(v.get("arguments"), Some(a) if !a.is_object()) {
                    return Err(ErrorCode::MalformedToolCall);
                }
                Payload::ToolRequest(de(v)?)
            }
            "capability_request" => Payload::CapabilityRequest(de(v)?),
            "decision" => Payload::Decision(de(v)?),
            "result" => Payload::Result(de(v)?),
            "event" => Payload::Event(de(v)?),
            "catalog" => Payload::Catalog(de(v)?),
            "selection" => Payload::Selection(de(v)?),
            "probe_report" => Payload::ProbeReport(de(v)?),
            _ => return Err(ErrorCode::MalformedEnvelope),
        })
    }
}

/// Provenance of a canonical intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub dialect: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub dialect_version: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub model: Option<String>,
    pub parser_version: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub source_turn: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub source_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub source_digest: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub raw_name: Option<String>,
    /// `"none"` or `"json_guess"`.
    #[serde(default, skip_serializing_if = "is_none")]
    pub coercion: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `tool_request`: what the model asked for. Carries no authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRequest {
    pub kind: ToolRequestKind,
    pub request_id: String,
    pub tool: ToolRef,
    pub arguments: Map<String, Value>,
    pub provenance: Provenance,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `input.schema.json`: one piece of material that was placed in front of the model (0.3 cut P2).
/// `trust` is assigned by the runtime, never by the model. `parent_id` is serialized as an
/// explicit `null` for host-registered inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    pub input_id: String,
    pub content_kind: ContentKind,
    pub trust: TrustLevel,
    pub source: Party,
    pub origin: String,
    pub content_digest: String,
    pub trace_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `provenance.exposure` on a `tool_request` (0.3 cut P2): the inputs visible to the model when it
/// produced the turn and the least trusted level among them. The pipeline computes it; a value
/// arriving from a model or an envelope is never read (cut P3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exposure {
    pub inputs: Vec<String>,
    pub floor: TrustLevel,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// How a capability request was mapped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingInfo {
    pub table_version: String,
    pub rule_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub passthrough: bool,
    /// Digest of the runtime catalog the table was built against; checked before `decide`.
    #[serde(default, skip_serializing_if = "is_none")]
    pub catalog_digest: Option<String>,
    /// Argument keys CrossAxis coerced against the capability schema.
    #[serde(default, skip_serializing_if = "is_none")]
    pub coerced: Option<Vec<String>>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Typed runtime-specific metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExtension {
    pub vocabulary: String,
    pub values: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `capability_request`: a canonical intent in one runtime's coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub kind: CapabilityRequestKind,
    pub request_id: String,
    pub runtime: String,
    pub capability: String,
    pub arguments: Map<String, Value>,
    pub tool: ToolRef,
    pub mapping: MappingInfo,
    #[serde(default, skip_serializing_if = "is_none")]
    pub runtime_effects: Option<RuntimeExtension>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Who decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authority {
    pub runtime: String,
    pub policy_engine: String,
    #[serde(default)]
    pub decision_id: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Approval handle minted by the runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub approval_id: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// The runtime's decision about one capability request. Only a runtime produces one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub kind: DecisionMsgKind,
    pub request_id: String,
    pub decision: DecisionKind,
    #[serde(default)]
    pub capability: Option<String>,
    pub authority: Authority,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub approval: Option<Approval>,
    #[serde(default)]
    pub runtime_state: Option<RuntimeExtension>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Decision {
    /// True only for a known `authorized`. Unknown values are never authorization.
    pub fn is_authorized(&self) -> bool {
        self.decision == DecisionKind::Authorized
    }
}

/// `result.error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "is_none")]
    pub details: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl ErrorObject {
    pub fn new(code: ErrorCode, message: &str) -> Self {
        let retryable = matches!(
            code,
            ErrorCode::ExecutionTimeout | ErrorCode::RuntimeUnavailable
        );
        Self {
            code,
            message: message.chars().take(4096).collect(),
            retryable,
            details: None,
            extensions: Map::new(),
        }
    }
}

/// `result.provenance`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultProvenance {
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub content_kind: Option<ContentKind>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub trust: Option<TrustLevel>,
    #[serde(default)]
    pub trusted: Option<bool>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `result`: what returns to the model side for one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub kind: ResultKind,
    /// `None` only when rejection happened before any id existed.
    pub request_id: Option<String>,
    pub status: ResultStatus,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub error: Option<ErrorObject>,
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub provenance: Option<ResultProvenance>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl ToolResult {
    /// A successful result.
    pub fn ok(request_id: &str, data: Value) -> Self {
        Self::build(request_id, ResultStatus::Ok, data, None)
    }
    /// A non-ok result with an error.
    pub fn failed(
        request_id: Option<&str>,
        status: ResultStatus,
        code: ErrorCode,
        message: &str,
    ) -> Self {
        let mut r = Self::build(
            "",
            status,
            Value::Null,
            Some(ErrorObject::new(code, message)),
        );
        r.request_id = request_id.map(str::to_string);
        r
    }
    fn build(
        request_id: &str,
        status: ResultStatus,
        data: Value,
        error: Option<ErrorObject>,
    ) -> Self {
        Self {
            kind: ResultKind,
            request_id: Some(request_id.to_string()),
            status,
            data,
            error,
            decision: None,
            provenance: Some(ResultProvenance::default()),
            extensions: Map::new(),
        }
    }
    /// Digest of the canonical result with `provenance.duration_ms` forced to null (CORE.md).
    pub fn result_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("result serializes");
        if let Some(p) = v.get_mut("provenance").and_then(Value::as_object_mut) {
            p.insert("duration_ms".into(), Value::Null);
        }
        digest(&v)
    }
}

/// One RelayLine event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventMsgKind,
    pub event: EventKind,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub turn: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub payload: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// A capability a runtime exposes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub canonical: Option<ToolRef>,
    pub description: String,
    pub parameters: Map<String, Value>,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub runtime_effects: Option<RuntimeExtension>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub schema_digest: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// What one runtime exposes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub kind: CatalogKind,
    pub runtime: String,
    pub catalog_version: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub catalog_digest: Option<String>,
    pub capabilities: Vec<CapabilityDescriptor>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Catalog {
    /// Order-independent SHA-256 over capability names and schema digests: the sorted list of
    /// `[name, schema_digest or digest(parameters)]` pairs, canonicalized.
    /// True when `name` is advertised more than once with different definitions (the description
    /// or the parameters differ), so a call cannot be tied to one definition.
    pub fn is_ambiguous(&self, name: &str) -> bool {
        let defs: std::collections::HashSet<String> = self
            .capabilities
            .iter()
            .filter(|c| c.name == name)
            .map(|c| {
                canonicalize(&serde_json::json!({
                    "description": c.description,
                    "parameters": c.parameters,
                }))
            })
            .collect();
        defs.len() > 1
    }

    pub fn compute_digest(&self) -> String {
        let mut pairs: Vec<(String, String)> = self
            .capabilities
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.schema_digest
                        .clone()
                        .unwrap_or_else(|| digest(&Value::Object(c.parameters.clone()))),
                )
            })
            .collect();
        pairs.sort();
        digest(&Value::Array(
            pairs
                .into_iter()
                .map(|(a, b)| Value::Array(vec![a.into(), b.into()]))
                .collect(),
        ))
    }
}

/// Selector identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectorInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub max_capabilities: Option<u64>,
    /// Expansion bounds, recorded by the first `expand` (spec/CROSSAXIS.md).
    #[serde(default, skip_serializing_if = "is_none")]
    pub expansion: Option<ExpansionBounds>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Bounds recorded in `selector.expansion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionBounds {
    pub max_expansions: u64,
    pub max_added_per_expansion: u64,
}

/// A capability refused by an expansion, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefusedEntry {
    pub name: String,
    pub reason: String,
}

/// One entry of `selection.expansions[]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpansionEntry {
    pub round: u64,
    pub reason: String,
    pub evidence_digest: String,
    pub added: Vec<String>,
    pub refused: Vec<RefusedEntry>,
}

/// A selected capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedEntry {
    pub name: String,
    pub rule_id: String,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub domains: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// An excluded capability and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExcludedEntry {
    pub name: String,
    pub reason: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// A `{full, selected}` pair of non-negative integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullSelected {
    pub full: u64,
    pub selected: u64,
}

/// `measure.tokens`: every number is in `unit`; units are never mixed in one block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenMeasure {
    /// `bytes`, `tokens_model_reported`, `tokens_endpoint_tokenizer` or `tokens_estimated`.
    pub unit: String,
    pub source: String,
    pub base_context: u64,
    pub tool_schema: FullSelected,
    pub first_turn_prompt: FullSelected,
    #[serde(default, skip_serializing_if = "is_none")]
    pub all_rounds_prompt: Option<FullSelected>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub completion: Option<FullSelected>,
}

/// Before/after size of the rendered tool surface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    #[serde(default, skip_serializing_if = "is_none")]
    pub full_count: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub selected_count: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub full_rendered_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub selected_rendered_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub full_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub selected_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub tokenizer: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub rendered_bytes: Option<FullSelected>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub tokens: Option<TokenMeasure>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// The CrossAxis Select receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub kind: SelectionKind,
    pub runtime: String,
    pub catalog_digest: String,
    pub selector: SelectorInfo,
    pub requested_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub always_include: Vec<String>,
    pub selected: Vec<SelectedEntry>,
    pub excluded: Vec<ExcludedEntry>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub measure: Option<Measure>,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub expansions: Vec<ExpansionEntry>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub parent_digest: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub selection_digest: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// One probe verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub name: String,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub attempts: Option<u64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub request_digest: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub response_digest: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// A profile claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileResult {
    pub name: String,
    pub status: ProfileStatus,
    pub required_probes: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Enough to reproduce a probe.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default)]
    pub hardware: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub endpoint_type: Option<String>,
    #[serde(default)]
    pub interplane_version: Option<String>,
    #[serde(default)]
    pub fixture_revision: Option<String>,
    #[serde(default)]
    pub model_revision: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Measured capabilities of one endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub kind: ProbeReportKind,
    pub probe_version: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub backend_version: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub model_digest: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub finished_at: Option<String>,
    pub probes: Vec<ProbeResult>,
    pub profiles: Vec<ProfileResult>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub environment: Option<Environment>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl TrustLevel {
    /// The `result.provenance.trusted` flag derived from a level: true only for `trusted_runtime`,
    /// false for the two untrusted levels, null otherwise.
    pub fn trusted_flag(&self) -> Option<bool> {
        match self {
            TrustLevel::TrustedRuntime => Some(true),
            TrustLevel::WorkspaceUntrusted
            | TrustLevel::ExternalUntrusted
            | TrustLevel::Unknown(_) => Some(false),
            TrustLevel::UserSupplied | TrustLevel::Undetermined => None,
        }
    }
}
