//! Dialect `openai_stream` (version 1): a streamed OpenAI-compatible response body (SSE text).
//! The fragments are assembled into one assistant message, which is parsed with the `openai`
//! rules. A stream that did not finish cleanly is rejected call by call, never guessed.
use std::collections::BTreeMap;

use interplane_core::{digest, ErrorCode, ToolRequest, ToolResult};
use serde_json::{Map, Value};

use crate::openai::OpenAi;
use crate::*;

pub struct OpenAiStream;

const NAME: &str = "openai_stream";
const COMPLETE: [&str; 3] = ["stop", "tool_calls", "function_call"];

/// The data of each complete SSE event, in order.
pub fn events(body: &str) -> Vec<String> {
    let (mut out, mut data) = (Vec::new(), Vec::<&str>::new());
    for line in body.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
            data.clear();
        } else if line == "data" {
            data.push("");
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    out
}

#[derive(Default)]
struct Slot {
    id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

fn keep_first(slot: &mut Option<String>, value: Option<&Value>, what: &str) -> Result<(), String> {
    let Some(v) = value.and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    match slot {
        None => *slot = Some(v.to_string()),
        Some(old) if old != v => return Err(format!("conflicting tool call {what}")),
        Some(_) => {}
    }
    Ok(())
}

/// `(message, finish_reason)` for a stream body, or the stream error.
pub fn assemble(body: &str) -> Result<(Value, Option<String>), String> {
    let (mut text, mut reasoning) = (String::new(), String::new());
    let mut any_reasoning = false;
    let mut calls: BTreeMap<u64, Slot> = BTreeMap::new();
    let mut finish: Option<String> = None;
    let mut done = false;
    for data in events(body) {
        if done {
            return Err("event after [DONE]".into());
        }
        if data == "[DONE]" {
            done = true;
            continue;
        }
        let chunk = match serde_json::from_str::<Value>(&data) {
            Ok(Value::Object(m)) => m,
            _ => return Err("stream chunk is not a JSON object".into()),
        };
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            continue;
        };
        for choice in choices {
            let Some(choice) = choice.as_object() else {
                return Err("stream choice is not an object".into());
            };
            if choice.get("index").is_some_and(|i| i.as_u64() != Some(0)) {
                return Err("multiple choices are not supported".into());
            }
            if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                if let Some(s) = delta.get("content").and_then(Value::as_str) {
                    text.push_str(s);
                }
                for key in ["reasoning_content", "reasoning"] {
                    if let Some(s) = delta.get(key).and_then(Value::as_str) {
                        reasoning.push_str(s);
                        any_reasoning = true;
                    }
                }
                for frag in delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(index) = frag.get("index").and_then(Value::as_u64) else {
                        return Err("tool call fragment has no index".into());
                    };
                    let slot = calls.entry(index).or_default();
                    keep_first(&mut slot.id, frag.get("id"), "id")?;
                    if let Some(function) = frag.get("function").and_then(Value::as_object) {
                        keep_first(&mut slot.name, function.get("name"), "name")?;
                        if let Some(s) = function.get("arguments").and_then(Value::as_str) {
                            slot.arguments.get_or_insert_with(String::new).push_str(s);
                        }
                    }
                }
            }
            match choice.get("finish_reason") {
                None | Some(Value::Null) => {}
                Some(Value::String(r)) => match &finish {
                    None => finish = Some(r.clone()),
                    Some(f) if f != r => return Err("conflicting finish_reason".into()),
                    Some(_) => {}
                },
                Some(_) => return Err("finish_reason is not a string".into()),
            }
        }
    }
    let mut message = Map::new();
    message.insert("role".into(), "assistant".into());
    message.insert("content".into(), text.into());
    if any_reasoning {
        message.insert("reasoning_content".into(), reasoning.into());
    }
    if !calls.is_empty() {
        let out: Vec<Value> = calls
            .into_values()
            .map(|s| {
                let mut function = Map::new();
                if let Some(n) = s.name {
                    function.insert("name".into(), n.into());
                }
                if let Some(a) = s.arguments {
                    function.insert("arguments".into(), a.into());
                }
                let mut call = Map::new();
                call.insert("type".into(), "function".into());
                call.insert("function".into(), function.into());
                if let Some(id) = s.id {
                    call.insert("id".into(), id.into());
                }
                Value::Object(call)
            })
            .collect();
        message.insert("tool_calls".into(), out.into());
    }
    Ok((Value::Object(message), finish))
}

impl Dialect for OpenAiStream {
    fn name(&self) -> &str {
        NAME
    }
    fn version(&self) -> &str {
        "1"
    }

    fn parse(&self, input: &Value, ctx: &ParseContext) -> LenshiftTurn {
        let assembled = match input.as_str() {
            Some(body) => assemble(body),
            None => Err("stream is not text".into()),
        };
        let (message, finish) = match assembled {
            Ok(v) => v,
            Err(e) => {
                let source_digest = match input.as_str() {
                    Some(body) => text_digest(body),
                    None => digest(input),
                };
                return LenshiftTurn {
                    dialect: NAME.into(),
                    dialect_version: "1".into(),
                    parser_version: PARSER_VERSION.into(),
                    model: ctx.model.clone(),
                    text: String::new(),
                    reasoning_digest: None,
                    intents: vec![],
                    rejected: vec![RejectedCall {
                        index: 0,
                        code: ErrorCode::MalformedToolCall,
                        message: e,
                        source_digest: Some(source_digest),
                        request_id: assign_request_id(None, ctx, 0),
                    }],
                    partial: false,
                    intent_indices: vec![],
                };
            }
        };
        let mut turn = OpenAi.parse(&message, ctx);
        turn.dialect = NAME.into();
        for intent in &mut turn.intents {
            intent.provenance.dialect = NAME.into();
        }
        if finish.as_deref().is_some_and(|f| COMPLETE.contains(&f)) {
            return turn;
        }
        let intents = std::mem::take(&mut turn.intents);
        let indices = std::mem::take(&mut turn.intent_indices);
        let mut cut: Vec<RejectedCall> = indices
            .into_iter()
            .zip(intents)
            .map(|(index, i)| RejectedCall {
                index,
                code: ErrorCode::MalformedToolCall,
                message: String::new(),
                source_digest: i.provenance.source_digest,
                request_id: i.request_id,
            })
            .chain(std::mem::take(&mut turn.rejected))
            .collect();
        cut.sort_by_key(|r| r.index);
        for r in &mut cut {
            r.code = ErrorCode::MalformedToolCall;
            r.message = "truncated tool call".into();
        }
        turn.partial = !cut.is_empty();
        turn.rejected = cut;
        turn
    }

    fn render_result(&self, result: &ToolResult, intent: Option<&ToolRequest>) -> Value {
        OpenAi.render_result(result, intent)
    }
}
