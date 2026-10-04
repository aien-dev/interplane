//! Dialect `qwen35` (version 1): the Qwen3.5 chat-template tool-call grammar as text.
use interplane_core::{ErrorCode, ToolRequest, ToolResult};
use serde_json::{Map, Value};

use crate::*;

pub struct Qwen35;

const TC_OPEN: &str = "<tool_call>";
const TC_CLOSE: &str = "</tool_call>";

/// How a body parse can fail.
enum BodyError {
    Truncated,
    Malformed(String),
}

struct Call {
    name: String,
    args: Map<String, Value>,
    hermes: bool,
}

/// Strip exactly one leading and one trailing newline (the template emits `\n` + value + `\n`).
fn strip_one_newline(v: &str) -> &str {
    let v = v.strip_prefix('\n').unwrap_or(v);
    v.strip_suffix('\n').unwrap_or(v)
}

fn json_guess(v: &str) -> Value {
    match serde_json::from_str::<Value>(v) {
        Ok(Value::String(_)) | Err(_) => Value::String(v.to_string()),
        Ok(parsed) => parsed,
    }
}

fn parse_body(body: &str) -> Result<Call, BodyError> {
    let b = body.trim();
    if let Some(rest) = b.strip_prefix("<function=") {
        let gt = rest.find('>').ok_or(BodyError::Truncated)?;
        let name = rest[..gt].trim().to_string();
        if name.is_empty() {
            return Err(BodyError::Malformed("empty function name".into()));
        }
        let mut rest = &rest[gt + 1..];
        let mut args = Map::new();
        loop {
            rest = rest.trim_start();
            if rest.starts_with("</function>") {
                return Ok(Call {
                    name,
                    args,
                    hermes: false,
                });
            }
            if let Some(p) = rest.strip_prefix("<parameter=") {
                let gt = p.find('>').ok_or(BodyError::Truncated)?;
                let key = p[..gt].trim().to_string();
                let after = &p[gt + 1..];
                let end = after.find("</parameter>").ok_or(BodyError::Truncated)?;
                if key.is_empty() {
                    return Err(BodyError::Malformed("empty parameter name".into()));
                }
                if args.contains_key(&key) {
                    return Err(BodyError::Malformed(format!("duplicate parameter: {key}")));
                }
                args.insert(key, json_guess(strip_one_newline(&after[..end])));
                rest = &after[end + "</parameter>".len()..];
            } else if rest.is_empty() {
                return Err(BodyError::Truncated);
            } else {
                return Err(BodyError::Malformed(
                    "unexpected content in function body".into(),
                ));
            }
        }
    }
    if b.starts_with('{') {
        let v: Value = serde_json::from_str(b)
            .map_err(|_| BodyError::Malformed("tool call body is not valid JSON".into()))?;
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            return Err(BodyError::Malformed("missing tool name".into()));
        }
        let args = match v.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(m)) => m,
                _ => return Err(BodyError::Malformed("arguments is not an object".into())),
            },
            Some(_) => return Err(BodyError::Malformed("arguments is not an object".into())),
        };
        return Ok(Call {
            name,
            args,
            hermes: true,
        });
    }
    if b.starts_with("<function") {
        // `<function=` never completed.
        return Err(BodyError::Truncated);
    }
    Err(BodyError::Malformed("unrecognized tool call body".into()))
}

impl Dialect for Qwen35 {
    fn name(&self) -> &str {
        "qwen35"
    }
    fn version(&self) -> &str {
        "1"
    }

    fn parse(&self, input: &Value, ctx: &ParseContext) -> LenshiftTurn {
        let mut turn = LenshiftTurn {
            dialect: "qwen35".into(),
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
        let Some(full) = input.as_str() else {
            turn.rejected.push(RejectedCall {
                index: 0,
                code: ErrorCode::MalformedToolCall,
                message: "input is not text".into(),
                source_digest: Some(interplane_core::digest(input)),
                request_id: assign_request_id(None, ctx, 0),
            });
            return turn;
        };

        // 1. Remove <think>...</think> blocks; digest the first one's content.
        let mut visible = String::new();
        let mut rest = full;
        let mut first_think: Option<&str> = None;
        while let Some(s) = rest.find("<think>") {
            let after = &rest[s + "<think>".len()..];
            match after.find("</think>") {
                Some(e) => {
                    visible.push_str(&rest[..s]);
                    first_think.get_or_insert(&after[..e]);
                    rest = &after[e + "</think>".len()..];
                }
                None => break,
            }
        }
        visible.push_str(rest);
        turn.reasoning_digest = first_think
            .map(|t| t.trim_matches('\n'))
            .filter(|t| !t.trim().is_empty())
            .map(text_digest);

        // 2. Walk tool calls.
        let mut text = String::new();
        let mut rest = visible.as_str();
        let mut index = 0usize;
        let mut trailing = false;
        while let Some(s) = rest.find(TC_OPEN) {
            text.push_str(&rest[..s]);
            let from_open = &rest[s..];
            let request_id = assign_request_id(None, ctx, index);
            let Some(e) = from_open.find(TC_CLOSE) else {
                turn.partial = true;
                turn.rejected.push(RejectedCall {
                    index,
                    code: ErrorCode::MalformedToolCall,
                    message: "truncated tool call".into(),
                    source_digest: Some(text_digest(from_open)),
                    request_id,
                });
                rest = "";
                break;
            };
            let block = &from_open[..e + TC_CLOSE.len()];
            let body = &block[TC_OPEN.len()..block.len() - TC_CLOSE.len()];
            let src = text_digest(block);
            match parse_body(body) {
                Ok(call) => match canonical_tool(&call.name) {
                    Some(tool) => {
                        let (ver, coercion) = if call.hermes {
                            ("hermes_json", "none")
                        } else {
                            ("1", "json_guess")
                        };
                        turn.intents.push(make_intent(
                            "qwen35", ver, ctx, request_id, tool, call.args, None, src, &call.name,
                            coercion,
                        ));
                        turn.intent_indices.push(index);
                    }
                    None => turn.rejected.push(RejectedCall {
                        index,
                        code: ErrorCode::MalformedToolCall,
                        message: "invalid tool name".into(),
                        source_digest: Some(src),
                        request_id,
                    }),
                },
                Err(err) => {
                    let message = match err {
                        BodyError::Truncated => {
                            turn.partial = true;
                            "truncated tool call".to_string()
                        }
                        BodyError::Malformed(m) => m,
                    };
                    turn.rejected.push(RejectedCall {
                        index,
                        code: ErrorCode::MalformedToolCall,
                        message,
                        source_digest: Some(src),
                        request_id,
                    });
                }
            }
            index += 1;
            rest = &from_open[e + TC_CLOSE.len()..];
            trailing = !rest.contains(TC_OPEN) && !rest.trim().is_empty();
        }
        text.push_str(rest);
        turn.text = text.trim().to_string();
        if trailing {
            for i in &mut turn.intents {
                i.provenance
                    .extensions
                    .insert("trailing_text".into(), Value::Bool(true));
            }
        }
        turn
    }

    fn render_result(&self, result: &ToolResult, _intent: Option<&ToolRequest>) -> Value {
        Value::String(format!(
            "<tool_response>\n{}\n</tool_response>",
            serialize_result(result)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interplane_core::{digest_bytes, ResultStatus};
    use serde_json::json;

    fn ctx() -> ParseContext {
        ParseContext {
            trace_id: "tr".into(),
            turn: 0,
            model: "Qwen/Qwen3.5-9B".into(),
        }
    }
    fn p(s: &str) -> LenshiftTurn {
        Qwen35.parse(&json!(s), &ctx())
    }
    const CALL: &str = "<tool_call>\n<function=read_file>\n<parameter=path>\n/a.txt\n</parameter>\n</function>\n</tool_call>";

    #[test]
    fn valid_single_call_and_source_digest() {
        let t = p(&format!("Let me look.\n\n{CALL}"));
        assert_eq!(t.text, "Let me look.");
        assert_eq!(t.intents.len(), 1);
        let i = &t.intents[0];
        assert_eq!(i.tool.name, "read_file");
        assert_eq!(i.arguments["path"], json!("/a.txt"));
        assert_eq!(
            i.provenance.source_digest.as_deref(),
            Some(digest_bytes(CALL.as_bytes()).as_str())
        );
        assert_eq!(i.provenance.coercion.as_deref(), Some("json_guess"));
        assert_eq!(i.request_id, "tr:t0:c0");
        assert!(!t.partial && t.rejected.is_empty());
    }
    #[test]
    fn multiple_calls_and_indices() {
        let t = p(&format!("{CALL}\n{CALL}"));
        assert_eq!(t.intents.len(), 2);
        assert_eq!(t.intent_indices, vec![0, 1]);
        assert_eq!(t.intents[1].request_id, "tr:t0:c1");
    }
    #[test]
    fn json_guess_and_multiline() {
        let t = p("<tool_call>\n<function=f>\n<parameter=n>\n42\n</parameter>\n<parameter=b>\ntrue\n</parameter>\n<parameter=s>\n\"quoted\"\n</parameter>\n<parameter=l>\nline1\nline2\n</parameter>\n<parameter=o>\n{\"a\": [1]}\n</parameter>\n<parameter=bad>\n{oops\n</parameter>\n</function>\n</tool_call>");
        let a = &t.intents[0].arguments;
        assert_eq!(a["n"], json!(42));
        assert_eq!(a["b"], json!(true));
        assert_eq!(a["s"], json!("\"quoted\""));
        assert_eq!(a["l"], json!("line1\nline2"));
        assert_eq!(a["o"], json!({"a": [1]}));
        assert_eq!(a["bad"], json!("{oops"));
    }
    #[test]
    fn reasoning_removed_and_digested() {
        let t = p(&format!(
            "<think>\nI should read.\n</think>\n\nOK\n\n{CALL}"
        ));
        assert_eq!(t.text, "OK");
        assert_eq!(t.reasoning_digest, Some(text_digest("I should read.")));
        let t = p("<think>\n\n</think>\n\nhello");
        assert_eq!((t.text.as_str(), t.reasoning_digest), ("hello", None));
    }
    #[test]
    fn truncated_variants_are_partial_and_rejected() {
        for s in [
            "<tool_call>\n<function=f>\n<parameter=a>\nx",
            "<tool_call>\n<function=f>\n<parameter=a>\nx\n</parameter>\n</tool_call>",
            "<tool_call>\n<function=f>\n</tool_call>",
            "text <tool_call>",
        ] {
            let t = p(s);
            assert!(t.partial, "{s}");
            assert!(t.intents.is_empty(), "{s}");
            assert_eq!(t.rejected.len(), 1, "{s}");
            assert_eq!(t.rejected[0].message, "truncated tool call");
        }
    }
    #[test]
    fn duplicate_key_and_empty_names_are_malformed_not_partial() {
        let t = p("<tool_call>\n<function=f>\n<parameter=a>\n1\n</parameter>\n<parameter=a>\n2\n</parameter>\n</function>\n</tool_call>");
        assert_eq!(t.rejected[0].message, "duplicate parameter: a");
        let t = p("<tool_call>\n<function=f>\n<parameter=a>\n1\n</parameter>\n<parameter=a>\n2\n</parameter>\n</function>\n</tool_call>");
        assert!(!t.partial && t.rejected.len() == 1);
        let t = p("<tool_call>\n<function=>\n</function>\n</tool_call>");
        assert_eq!(t.rejected.len(), 1);
        let t = p(
            "<tool_call>\n<function=f>\n<parameter=>\n1\n</parameter>\n</function>\n</tool_call>",
        );
        assert_eq!(t.rejected.len(), 1);
        let t = p("<tool_call>garbage</tool_call>");
        assert_eq!(t.rejected[0].code, ErrorCode::MalformedToolCall);
    }
    #[test]
    fn hermes_json_variant() {
        let t = p("<tool_call>\n{\"name\": \"filesystem.read\", \"arguments\": {\"path\": \"/x\", \"n\": 1}}\n</tool_call>");
        assert_eq!(t.dialect_version, "1");
        let i = &t.intents[0];
        assert_eq!(i.provenance.dialect_version.as_deref(), Some("hermes_json"));
        assert_eq!(i.tool.namespace.as_deref(), Some("filesystem"));
        assert_eq!(i.arguments["n"], json!(1));
        assert_eq!(i.provenance.coercion.as_deref(), Some("none"));
    }
    #[test]
    fn trailing_text_noted_not_an_error() {
        let t = p(&format!("{CALL}\nafter"));
        assert_eq!(t.text, "after");
        assert_eq!(
            t.intents[0].provenance.extensions["trailing_text"],
            json!(true)
        );
        assert!(t.rejected.is_empty());
    }
    #[test]
    fn plain_and_unknown_tool_parse_fine() {
        let t = p("The answer is 4.");
        assert_eq!((t.text.as_str(), t.intents.len()), ("The answer is 4.", 0));
        let t = p("<tool_call>\n<function=nonexistent.thing>\n</function>\n</tool_call>");
        assert_eq!(t.intents[0].tool.name, "thing");
        assert_eq!(Qwen35.parse(&json!(5), &ctx()).rejected.len(), 1);
    }
    #[test]
    fn render_wraps_in_tool_response() {
        let r = ToolResult::ok("r", json!({"a": 1}));
        assert_eq!(
            Qwen35.render_result(&r, None),
            json!("<tool_response>\n{\"a\":1}\n</tool_response>")
        );
        let e = ToolResult::failed(
            Some("r"),
            ResultStatus::Rejected,
            ErrorCode::MalformedToolCall,
            "m",
        );
        assert!(Qwen35
            .render_result(&e, None)
            .as_str()
            .unwrap()
            .contains("\"status\":\"rejected\""));
    }
}
