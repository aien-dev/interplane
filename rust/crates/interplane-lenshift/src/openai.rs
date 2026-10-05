//! Dialect `openai` (version 1): OpenAI-compatible assistant messages.
use interplane_core::{digest, ErrorCode, ToolRequest, ToolResult};
use serde_json::{json, Value};

use crate::*;

pub struct OpenAi;

impl Dialect for OpenAi {
    fn name(&self) -> &str {
        "openai"
    }
    fn version(&self) -> &str {
        "1"
    }

    fn parse(&self, input: &Value, ctx: &ParseContext) -> LenshiftTurn {
        let mut turn = LenshiftTurn {
            dialect: "openai".into(),
            dialect_version: "1".into(),
            parser_version: PARSER_VERSION.into(),
            model: ctx.model.clone(),
            text: String::new(),
            reasoning_digest: None,
            intents: vec![],
            rejected: vec![],
            partial: false,
            intent_indices: vec![],
        };
        let Some(msg) = input.as_object() else {
            turn.rejected.push(RejectedCall {
                index: 0,
                code: ErrorCode::MalformedToolCall,
                message: "assistant message is not an object".into(),
                source_digest: Some(digest(input)),
                request_id: assign_request_id(None, ctx, 0),
            });
            return turn;
        };
        if let Some(s) = msg.get("content").and_then(Value::as_str) {
            turn.text = s.to_string();
        }
        turn.reasoning_digest = ["reasoning_content", "reasoning"]
            .iter()
            .filter_map(|k| msg.get(*k).and_then(Value::as_str))
            .find(|s| !s.is_empty())
            .map(text_digest);

        let calls: Vec<&Value> = match msg.get("tool_calls").and_then(Value::as_array) {
            Some(a) if !a.is_empty() => a.iter().collect(),
            _ => msg
                .get("function_call")
                .filter(|v| v.is_object())
                .into_iter()
                .collect(),
        };
        let legacy = msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_none_or(|a| a.is_empty());
        for (index, call) in calls.into_iter().enumerate() {
            let src_digest = digest(call);
            let (id, func): (Option<&str>, Option<&Value>) = if legacy {
                (None, Some(call))
            } else {
                (call.get("id").and_then(Value::as_str), call.get("function"))
            };
            let request_id = assign_request_id(id, ctx, index);
            let reject = |turn: &mut LenshiftTurn, msg: &str| {
                turn.rejected.push(RejectedCall {
                    index,
                    code: ErrorCode::MalformedToolCall,
                    message: msg.into(),
                    source_digest: Some(src_digest.clone()),
                    request_id: request_id.clone(),
                });
            };
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if name.is_empty() {
                reject(&mut turn, "missing tool name");
                continue;
            }
            let Some(tool) = canonical_tool(name) else {
                reject(&mut turn, "invalid tool name");
                continue;
            };
            let args = match func.and_then(|f| f.get("arguments")) {
                Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
                    Ok(Value::Object(m)) => m,
                    Ok(_) => {
                        reject(&mut turn, "arguments is not an object");
                        continue;
                    }
                    Err(_) => {
                        reject(&mut turn, "arguments is not valid JSON");
                        continue;
                    }
                },
                // `arguments` must be a JSON string (spec/LENSHIFT.md). An already-decoded object is
                // what Ollama's own /api/chat sends, not an OpenAI-compatible endpoint.
                _ => {
                    reject(&mut turn, "arguments is not valid JSON");
                    continue;
                }
            };
            turn.intents.push(make_intent(
                "openai",
                "1",
                ctx,
                request_id,
                tool,
                args,
                id.map(str::to_string),
                src_digest.clone(),
                name,
                "none",
            ));
            turn.intent_indices.push(index);
        }
        turn
    }

    fn render_result(&self, result: &ToolResult, intent: Option<&ToolRequest>) -> Value {
        let id = intent
            .and_then(|i| i.provenance.source_call_id.clone())
            .or_else(|| result.request_id.clone())
            .unwrap_or_default();
        json!({"role": "tool", "tool_call_id": id, "content": serialize_result(result)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interplane_core::ResultStatus;

    fn ctx() -> ParseContext {
        ParseContext {
            trace_id: "tr".into(),
            turn: 2,
            model: "m".into(),
        }
    }
    fn call(id: Option<&str>, name: &str, args: Value) -> Value {
        let mut c = json!({"type": "function", "function": {"name": name, "arguments": args}});
        if let Some(i) = id {
            c["id"] = json!(i);
        }
        c
    }

    #[test]
    fn single_call_with_digest_of_canonical_call() {
        let c = call(
            Some("call_1"),
            "filesystem.read",
            json!("{\"path\": \"/a\"}"),
        );
        let t = OpenAi.parse(
            &json!({"role": "assistant", "content": null, "tool_calls": [c.clone()]}),
            &ctx(),
        );
        assert_eq!(t.intents.len(), 1);
        let i = &t.intents[0];
        assert_eq!(i.request_id, "call_1");
        assert_eq!(i.tool.namespace.as_deref(), Some("filesystem"));
        assert_eq!(i.arguments["path"], json!("/a"));
        assert_eq!(
            i.provenance.source_digest.as_deref(),
            Some(digest(&c).as_str())
        );
        assert_eq!(i.provenance.coercion.as_deref(), Some("none"));
        assert_eq!(t.text, "");
    }
    #[test]
    fn malformed_arguments_and_missing_name() {
        let msg = json!({"content": "hi", "tool_calls": [
            call(Some("a"), "x", json!("not json")),
            call(Some("b"), "x", json!("[1]")),
            call(Some("c"), "", json!("{}")),
            call(None, "ok", json!("{}")),
        ]});
        let t = OpenAi.parse(&msg, &ctx());
        let m: Vec<&str> = t.rejected.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            m,
            [
                "arguments is not valid JSON",
                "arguments is not an object",
                "missing tool name"
            ]
        );
        assert_eq!(t.intents[0].request_id, "tr:t2:c3");
        assert_eq!(t.intent_indices, vec![3]);
        assert_eq!(t.text, "hi");
    }
    #[test]
    fn reasoning_and_legacy_function_call() {
        let t = OpenAi.parse(
            &json!({"content": "x", "reasoning_content": "thought", "function_call": {"name": "f", "arguments": "{}"}}),
            &ctx(),
        );
        assert_eq!(t.reasoning_digest, Some(text_digest("thought")));
        assert_eq!(t.intents.len(), 1);
        assert_eq!(t.intents[0].provenance.source_call_id, None);
        assert_eq!(t.intents[0].request_id, "tr:t2:c0");
    }
    #[test]
    fn plain_answer_and_garbage_input() {
        let t = OpenAi.parse(&json!({"content": "just text"}), &ctx());
        assert!(t.intents.is_empty() && t.rejected.is_empty() && !t.partial);
        let t = OpenAi.parse(&json!("nope"), &ctx());
        assert_eq!(t.rejected.len(), 1);
    }
    #[test]
    fn render_ok_and_error() {
        let r = ToolResult::ok("r1", json!({"b": 1, "a": 2}));
        let v = OpenAi.render_result(&r, None);
        assert_eq!(
            v,
            json!({"role": "tool", "tool_call_id": "r1", "content": "{\"a\":2,\"b\":1}"})
        );
        let e = ToolResult::failed(
            Some("r1"),
            ResultStatus::Denied,
            ErrorCode::PolicyDenied,
            "no",
        );
        let v = OpenAi.render_result(&e, None);
        assert_eq!(
            v["content"],
            json!(
                "{\"error\":{\"code\":\"policy_denied\",\"message\":\"no\"},\"status\":\"denied\"}"
            )
        );
    }
}
