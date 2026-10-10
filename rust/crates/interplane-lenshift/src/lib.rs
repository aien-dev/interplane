//! Lenshift: model-native representation <-> canonical intent. Never executes, never authorizes.
use std::collections::BTreeMap;

use interplane_core::{
    canonicalize, digest_bytes, is_valid_id, ErrorCode, Provenance, ToolRef, ToolRequest,
    ToolRequestKind, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub mod aien_legacy;
pub mod ajax;
pub mod openai;
pub mod openai_stream;
pub mod qwen35;
pub mod replay;

/// What a dialect needs to know to fill request ids and provenance.
#[derive(Debug, Clone)]
pub struct ParseContext {
    pub trace_id: String,
    pub turn: u64,
    pub model: String,
}

/// A malformed call. It also becomes a `rejected` / `malformed_tool_call` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedCall {
    pub index: usize,
    pub code: ErrorCode,
    pub message: String,
    pub source_digest: Option<String>,
    /// The id the pipeline uses for the rejected call's result (source call id when valid,
    /// otherwise `<trace_id>:t<turn>:c<index>`).
    pub request_id: String,
}

/// The parsed result of one model turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LenshiftTurn {
    pub dialect: String,
    pub dialect_version: String,
    pub parser_version: String,
    pub model: String,
    pub text: String,
    pub reasoning_digest: Option<String>,
    pub intents: Vec<ToolRequest>,
    pub rejected: Vec<RejectedCall>,
    pub partial: bool,
    /// Emission index of each entry of `intents` (for interleaving with `rejected`).
    #[serde(default)]
    pub intent_indices: Vec<usize>,
}

/// A model dialect.
pub trait Dialect {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn parse(&self, input: &Value, ctx: &ParseContext) -> LenshiftTurn;
    /// Render a result as the dialect-native tool message. `intent` supplies the native call id.
    fn render_result(&self, result: &ToolResult, intent: Option<&ToolRequest>) -> Value;
}

/// Parser version recorded in provenance.
pub const PARSER_VERSION: &str = "1.0.0";

/// Dialects keyed by name. Registering a new one needs no Core change.
#[derive(Default)]
pub struct DialectRegistry {
    dialects: BTreeMap<String, Box<dyn Dialect>>,
}

impl DialectRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    /// `aien_legacy`, `openai`, `openai_stream` and `qwen35`. `ajax` is reserved and deliberately absent.
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(aien_legacy::AienLegacy));
        r.register(Box::new(openai::OpenAi));
        r.register(Box::new(openai_stream::OpenAiStream));
        r.register(Box::new(qwen35::Qwen35));
        r
    }
    pub fn register(&mut self, d: Box<dyn Dialect>) {
        self.dialects.insert(d.name().to_string(), d);
    }
    /// Unknown name -> `unsupported_dialect`.
    pub fn get(&self, name: &str) -> Result<&dyn Dialect, ErrorCode> {
        self.dialects
            .get(name)
            .map(|b| b.as_ref())
            .ok_or(ErrorCode::UnsupportedDialect)
    }
    pub fn names(&self) -> Vec<&str> {
        self.dialects.keys().map(String::as_str).collect()
    }
}

/// Split `raw` on the last `.` into namespace + name only when the prefix matches
/// `^[a-z][a-z0-9_]*$`. Returns `None` when the name is not a legal `ToolRef` name.
pub fn canonical_tool(raw: &str) -> Option<ToolRef> {
    fn name_ok(n: &str) -> bool {
        !n.is_empty()
            && n.len() <= 128
            && n.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
    }
    fn ns_ok(p: &str) -> bool {
        let mut b = p.bytes();
        matches!(b.next(), Some(c) if c.is_ascii_lowercase())
            && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    }
    if !name_ok(raw) {
        return None;
    }
    if let Some((pre, post)) = raw.rsplit_once('.') {
        if ns_ok(pre) && name_ok(post) {
            return Some(ToolRef {
                namespace: Some(pre.into()),
                name: post.into(),
            });
        }
    }
    Some(ToolRef {
        namespace: None,
        name: raw.into(),
    })
}

/// Request id: the dialect's call id when it is a valid Id, else `<trace>:t<turn>:c<index>`.
pub fn assign_request_id(call_id: Option<&str>, ctx: &ParseContext, index: usize) -> String {
    match call_id {
        Some(id) if is_valid_id(id) => id.to_string(),
        _ => format!("{}:t{}:c{}", ctx.trace_id, ctx.turn, index),
    }
}

/// Build a `ToolRequest` with filled provenance.
#[allow(clippy::too_many_arguments)]
pub fn make_intent(
    dialect: &str,
    dialect_version: &str,
    ctx: &ParseContext,
    request_id: String,
    tool: ToolRef,
    arguments: Map<String, Value>,
    source_call_id: Option<String>,
    source_digest: String,
    raw_name: &str,
    coercion: &str,
) -> ToolRequest {
    ToolRequest {
        kind: ToolRequestKind,
        request_id,
        tool,
        arguments,
        provenance: Provenance {
            dialect: dialect.into(),
            dialect_version: Some(dialect_version.into()),
            model: Some(ctx.model.clone()),
            parser_version: PARSER_VERSION.into(),
            source_turn: Some(ctx.turn),
            source_call_id,
            source_digest: Some(source_digest),
            raw_name: Some(raw_name.into()),
            coercion: Some(coercion.into()),
            extensions: Map::new(),
        },
        extensions: Map::new(),
    }
}

/// `sha256:` of UTF-8 text.
pub fn text_digest(s: &str) -> String {
    digest_bytes(s.as_bytes())
}

/// Shared result serialization: canonical `data` for ok, otherwise
/// `{"error": {"code","message"}, "status": ...}`.
pub fn serialize_result(result: &ToolResult) -> String {
    if result.status == interplane_core::ResultStatus::Ok {
        canonicalize(&result.data)
    } else {
        let (code, message) = match &result.error {
            Some(e) => (e.code.as_str().to_string(), e.message.clone()),
            None => ("execution_error".to_string(), String::new()),
        };
        canonicalize(
            &json!({"error": {"code": code, "message": message}, "status": result.status.as_str()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_split_on_last_dot() {
        assert_eq!(
            canonical_tool("filesystem.read").unwrap(),
            ToolRef {
                namespace: Some("filesystem".into()),
                name: "read".into()
            }
        );
        assert_eq!(canonical_tool("read_file").unwrap().namespace, None);
        assert_eq!(
            canonical_tool("a.b.c").unwrap(),
            ToolRef {
                namespace: None,
                name: "a.b.c".into()
            }
        );
        assert_eq!(canonical_tool("Files.read").unwrap().namespace, None);
        assert_eq!(canonical_tool("x.").unwrap().namespace, None);
        assert!(canonical_tool("has space").is_none());
        assert!(canonical_tool("").is_none());
    }

    #[test]
    fn unknown_dialect_is_unsupported() {
        let r = DialectRegistry::with_defaults();
        assert_eq!(
            r.names(),
            vec!["aien_legacy", "openai", "openai_stream", "qwen35"]
        );
        assert!(r.get("openai").is_ok());
        assert_eq!(
            r.get("frobnicate").err(),
            Some(ErrorCode::UnsupportedDialect)
        );
        assert_eq!(r.get("ajax").err(), Some(ErrorCode::UnsupportedDialect));
    }
}
