//! Dialect `aien_legacy` (version 1): aien-cli's textual `<tool_call>` JSON protocol.
//!
//! Ground truth: aien-sovereign-core 7580039, `crates/aien-cli/src/client.rs:350-419` (parser)
//! and `main.rs:330-340` (result reinjection). See `spec/LENSHIFT.md`.
use interplane_core::{ErrorCode, ToolRequest, ToolResult};
use regex::Regex;
use serde_json::{Map, Value};

use crate::*;

pub struct AienLegacy;

const TC_OPEN: &str = "<tool_call>";
const TC_CLOSE: &str = "</tool_call>";
/// ASCII whitespace, identical to the Python reference implementation.
const WS: &[char] = &[' ', '\t', '\n', '\r', '\x0b', '\x0c'];
const FENCE: &str = r#"(?s)```(?:json|tool_call)?[ \t\n\r\x0b\x0c]*(\{[ \t\n\r\x0b\x0c]*"name"[ \t\n\r\x0b\x0c]*:[ \t\n\r\x0b\x0c]*"[^"]+".*?\})[ \t\n\r\x0b\x0c]*```"#;

/// aien-cli `parse_tool_call_json` (client.rs:350-379). Returns the value and the repairs used.
fn repair_json(raw: &str) -> Result<(Value, Vec<&'static str>), String> {
    let body = raw.trim_matches(WS);
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return Ok((v, vec![]));
    }
    if let Some(stripped) = body.strip_suffix(']') {
        let fixed = format!("{stripped}}}");
        if let Ok(v) = serde_json::from_str::<Value>(&fixed) {
            return Ok((v, vec!["bracket_to_brace"]));
        }
    }
    let opens = body.chars().filter(|&c| c == '{').count();
    let closes = body.chars().filter(|&c| c == '}').count();
    if opens > closes {
        let mut candidate = body.to_string();
        if body.chars().filter(|&c| c == '"').count() % 2 == 1 {
            candidate.push('"');
        }
        candidate.push_str(&"}".repeat(opens - closes));
        if let Ok(v) = serde_json::from_str::<Value>(&candidate) {
            return Ok((v, vec!["close_braces"]));
        }
    }
    Err("tool call body is not valid JSON".into())
}

type Decoded = (String, Map<String, Value>, Vec<&'static str>);

fn decode(raw: &str) -> Result<Decoded, String> {
    let (obj, repairs) = repair_json(raw)?;
    let name = match obj.get("name").and_then(Value::as_str) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return Err("tool call JSON has no name".into()),
    };
    if name == "tool_name" {
        return Err("placeholder tool name".into());
    }
    let args = match obj.get("arguments") {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("arguments is not an object".into()),
    };
    Ok((name, args, repairs))
}

/// (body or None when truncated, source text, form)
type Block<'a> = (Option<&'a str>, &'a str, &'static str);

impl Dialect for AienLegacy {
    fn name(&self) -> &str {
        "aien_legacy"
    }
    fn version(&self) -> &str {
        "1"
    }

    fn parse(&self, input: &Value, ctx: &ParseContext) -> LenshiftTurn {
        let mut turn = LenshiftTurn {
            dialect: "aien_legacy".into(),
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
                message: "assistant turn is not text".into(),
                source_digest: None,
                request_id: assign_request_id(None, ctx, 0),
            });
            return turn;
        };

        // 1. A leading <think>...</think> block is removed and digested.
        let mut text = full;
        let lead = full.trim_start_matches(WS);
        if let Some(after) = lead.strip_prefix("<think>") {
            if let Some(e) = after.find("</think>") {
                let reasoning = after[..e].trim();
                turn.reasoning_digest = if reasoning.is_empty() {
                    None
                } else {
                    Some(text_digest(reasoning))
                };
                text = &after[e + "</think>".len()..];
            }
        }

        // 2. Split into call blocks. The fenced fallback only runs when no <tool_call> marker exists.
        let mut pieces = String::new();
        let mut blocks: Vec<Block> = vec![];
        if text.contains(TC_OPEN) {
            let mut rest = text;
            loop {
                let Some(s) = rest.find(TC_OPEN) else {
                    pieces.push_str(rest);
                    break;
                };
                pieces.push_str(&rest[..s]);
                let from_open = &rest[s..];
                match from_open[TC_OPEN.len()..].find(TC_CLOSE) {
                    None => {
                        turn.partial = true;
                        blocks.push((None, from_open, "tool_call"));
                        break;
                    }
                    Some(e) => {
                        let close_at = TC_OPEN.len() + e;
                        let stop = close_at + TC_CLOSE.len();
                        blocks.push((
                            Some(&from_open[TC_OPEN.len()..close_at]),
                            &from_open[..stop],
                            "tool_call",
                        ));
                        rest = &from_open[stop..];
                    }
                }
            }
        } else {
            let re = Regex::new(FENCE).expect("static regex");
            let mut pos = 0;
            for caps in re.captures_iter(text) {
                let m = caps.get(0).expect("whole match");
                pieces.push_str(&text[pos..m.start()]);
                blocks.push((
                    Some(caps.get(1).expect("group 1").as_str()),
                    m.as_str(),
                    "fenced_json",
                ));
                pos = m.end();
            }
            pieces.push_str(&text[pos..]);
        }
        turn.text = pieces.trim().to_string();

        for (index, (body, source, form)) in blocks.into_iter().enumerate() {
            let src = text_digest(source);
            let request_id = assign_request_id(None, ctx, index);
            let reject = |message: &str| RejectedCall {
                index,
                code: ErrorCode::MalformedToolCall,
                message: message.into(),
                source_digest: Some(src.clone()),
                request_id: request_id.clone(),
            };
            let Some(body) = body else {
                turn.rejected.push(reject("truncated tool call"));
                continue;
            };
            let (name, args, repairs) = match decode(body) {
                Ok(c) => c,
                Err(m) => {
                    turn.rejected.push(reject(&m));
                    continue;
                }
            };
            let Some(tool) = canonical_tool(&name) else {
                turn.rejected.push(reject("invalid tool name"));
                continue;
            };
            let mut intent = make_intent(
                "aien_legacy",
                "1",
                ctx,
                request_id.clone(),
                tool,
                args,
                None,
                src.clone(),
                &name,
                "none",
            );
            if !repairs.is_empty() {
                intent.provenance.extensions.insert(
                    "repairs".into(),
                    Value::Array(repairs.iter().map(|r| Value::String((*r).into())).collect()),
                );
            }
            if form != "tool_call" {
                intent
                    .provenance
                    .extensions
                    .insert("form".into(), Value::String(form.into()));
            }
            turn.intents.push(intent);
            turn.intent_indices.push(index);
        }
        turn
    }

    fn render_result(&self, result: &ToolResult, intent: Option<&ToolRequest>) -> Value {
        let name = intent
            .and_then(|i| i.provenance.raw_name.as_deref())
            .filter(|n| !n.is_empty())
            .unwrap_or("unknown");
        serde_json::json!({
            "role": "user",
            "content": format!(
                "<tool_response name=\"{name}\">\n{}\n</tool_response>",
                serialize_result(result)
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(s: &str) -> LenshiftTurn {
        let ctx = ParseContext {
            trace_id: "t".into(),
            turn: 0,
            model: "m".into(),
        };
        AienLegacy.parse(&json!(s), &ctx)
    }

    #[test]
    fn repairs_and_rejections() {
        let t = parse("<tool_call>\n{\"name\": \"f\", \"arguments\": {\"s\": \"x\n</tool_call>");
        assert_eq!(t.intents[0].arguments["s"], json!("x"));
        assert_eq!(
            t.intents[0].provenance.extensions["repairs"],
            json!(["close_braces"])
        );
        for body in [
            "{\"name\": \"f\",}",
            "{'name': 'f'}",
            "null",
            "[]",
            "{\"name\": \"\"}",
        ] {
            let t = parse(&format!("<tool_call>\n{body}\n</tool_call>"));
            assert!(
                t.intents.is_empty() && t.rejected.len() == 1 && !t.partial,
                "{body}"
            );
        }
        let t = parse("x <tool_call>\n{\"name\": \"f\"}");
        assert!(t.partial && t.intents.is_empty() && t.text == "x");
    }

    #[test]
    fn fence_only_without_tag_and_render_shape() {
        let fenced = "```json\n{\"name\": \"f\", \"arguments\": {}}\n```";
        let t = parse(&format!("a {fenced} b"));
        assert_eq!(
            t.intents[0].provenance.extensions["form"],
            json!("fenced_json")
        );
        assert_eq!(t.text, "a  b");
        let t = parse(&format!("<tool_call>\nbroken\n</tool_call>\n{fenced}"));
        assert!(t.intents.is_empty() && t.rejected.len() == 1);
    }
}
