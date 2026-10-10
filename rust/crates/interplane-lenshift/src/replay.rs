//! Sanitized offline replay of provider HTTP responses (issue #98).
//!
//! A capture records what a provider sent (status, ordered body chunks) in a scrubbed form. The
//! replay feeds those chunks through the existing dialect parsers exactly as live bytes would be
//! (concatenated in order, then parsed) and reduces the result to an [`Outcome`]. An outcome is
//! observed provider behaviour plus parsed requests. It never carries a decision or any
//! authority: authorization stays in the runtime, and a capture is never evidence of it.
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{DialectRegistry, ParseContext};

pub const FORMAT: &str = "interplane-replay/1";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// `synthetic` (hand written to model a behaviour) or `recorded` (scrubbed live capture).
    pub origin: String,
    pub source_commit: String,
    pub captured_on: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header NAMES only. A value (anything containing `:` or `=`) is refused by the scanner.
    pub header_names: Vec<String>,
    pub body: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub status: u16,
    pub header_names: Vec<String>,
    /// Transport chunks in arrival order. A chunk may end mid-event or mid-JSON.
    pub chunks: Vec<String>,
    /// Optional per-chunk delay in milliseconds. Informational; the replay never sleeps.
    #[serde(default)]
    pub timing_ms: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub name: String,
    pub arguments: Value,
}

/// The golden values and the replay result share this shape.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    /// `parsed`, `rate_limited`, `server_error`, `client_error` or `unparsable_body`.
    pub class: String,
    pub text: String,
    pub tool_calls: Vec<Call>,
    /// Lenshift rejection codes, in order.
    pub rejected: Vec<String>,
    pub partial: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub format: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    /// `chat_completions_stream` or `chat_completions`.
    pub endpoint_kind: String,
    pub provenance: Provenance,
    pub request: Request,
    pub response: Response,
    pub expect: Outcome,
}

fn empty(class: &str) -> Outcome {
    Outcome {
        class: class.into(),
        text: String::new(),
        tool_calls: vec![],
        rejected: vec![],
        partial: false,
    }
}

/// Feed a capture through the dialect parsers. Never sleeps, never touches the network.
pub fn replay(c: &Capture) -> Outcome {
    match c.response.status {
        429 => return empty("rate_limited"),
        500..=599 => return empty("server_error"),
        s if s >= 400 => return empty("client_error"),
        _ => {}
    }
    let body: String = c.response.chunks.concat();
    let (dialect, input) = if c.endpoint_kind == "chat_completions_stream" {
        ("openai_stream", Value::String(body))
    } else {
        match serde_json::from_str::<Value>(&body) {
            // A non-stream body is `{choices:[{message}]}`; the `openai` dialect takes the message.
            Ok(v) => match v.pointer("/choices/0/message") {
                Some(m) => ("openai", m.clone()),
                None => return empty("unparsable_body"),
            },
            Err(_) => return empty("unparsable_body"),
        }
    };
    let ctx = ParseContext {
        trace_id: "trace-replay".into(),
        turn: 0,
        model: c.model.clone(),
    };
    let reg = DialectRegistry::with_defaults();
    let t = reg
        .get(dialect)
        .expect("dialect registered")
        .parse(&input, &ctx);
    Outcome {
        class: "parsed".into(),
        text: t.text,
        tool_calls: t
            .intents
            .iter()
            .map(|i| Call {
                name: i.tool.name.clone(),
                arguments: Value::Object(i.arguments.clone()),
            })
            .collect(),
        rejected: t
            .rejected
            .iter()
            .map(|r| {
                json!(r.code)
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("{:?}", r.code))
            })
            .collect(),
        partial: t.partial,
    }
}

/// Replay and compare to the golden values. `Err` carries the actual outcome for the report.
pub fn check(c: &Capture) -> Result<(), String> {
    let got = replay(c);
    if got == c.expect {
        Ok(())
    } else {
        Err(format!(
            "{}: replay drifted from golden\n  want {}\n  got  {}",
            c.name,
            json!(c.expect),
            json!(got)
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// Scrubbing and scanning
// ---------------------------------------------------------------------------------------------

/// One class of sensitive-looking text.
struct Rule {
    class: &'static str,
    re: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        class: "bearer-token",
        re: r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}",
    },
    Rule {
        class: "sensitive-header-with-value",
        re: r"(?i)\b(authorization|proxy-authorization|cookie|set-cookie|x-api-key|api-key|x-auth-token)\s*[:=]\s*[A-Za-z0-9._~+/=-]{16,}",
    },
    Rule {
        class: "api-key",
        re: r"\b(sk|pk|rk)-[A-Za-z0-9_-]{16,}",
    },
    Rule {
        class: "aws-key",
        re: r"\b(AKIA|ASIA)[0-9A-Z]{16}\b",
    },
    Rule {
        class: "provider-token",
        re: r"\b(ghp|gho|ghs|xox[abp]|AIza)[-_A-Za-z0-9]{16,}",
    },
    Rule {
        class: "cookie-value",
        re: r"(?i)\b(cookie|set-cookie)\s*:\s*[^\s=;]+=\S+",
    },
    Rule {
        class: "jwt",
        re: r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*",
    },
    Rule {
        class: "pem-block",
        re: r"-----BEGIN [A-Z ]+-----",
    },
    Rule {
        class: "payment-key",
        re: r"\b(sk|pk|rk)_(live|test)_[A-Za-z0-9]{8,}",
    },
    Rule {
        class: "hf-token",
        re: r"\bhf_[A-Za-z0-9]{16,}",
    },
    Rule {
        class: "ip-address",
        re: r"\b(\d{1,3}\.){3}\d{1,3}\b",
    },
    Rule {
        class: "file-path",
        re: r"(/home/|/Users/|/root/|/etc/|/var/|/tmp/|\b[A-Za-z]:\\)",
    },
    Rule {
        class: "long-hex",
        re: r"\b[0-9a-fA-F]{32,}\b",
    },
    Rule {
        class: "long-base64",
        re: r"[A-Za-z0-9+/_-]{40,}={0,2}",
    },
    Rule {
        class: "email",
        re: r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}",
    },
    Rule {
        class: "tenant-id",
        re: r"(?i)\b(org|ws|workspace|tenant|acct|account)[-_][A-Za-z0-9]{8,}",
    },
    Rule {
        class: "query-secret",
        re: r"(?i)[?&](api[_-]?key|key|token|access_token|secret|sig|signature|password)=[^&\s\x22]{4,}",
    },
];

fn compiled() -> Vec<(&'static str, Regex)> {
    RULES
        .iter()
        .map(|r| (r.class, Regex::new(r.re).expect("rule compiles")))
        .collect()
}

fn placeholder() -> Regex {
    Regex::new(r"^<[A-Z0-9_ -]{1,40}>$").expect("placeholder")
}

fn sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    [
        "tenant_id",
        "workspace_id",
        "org_id",
        "organization_id",
        "account_id",
        "organization",
        "user_id",
    ]
    .iter()
    .any(|s| k == *s)
}

/// Replace every sensitive-looking span in `s` with `<REDACTED:class>`.
pub fn scrub_text(s: &str) -> String {
    let mut out = s.to_string();
    for (class, re) in compiled() {
        out = re
            .replace_all(&out, format!("<REDACTED:{class}>"))
            .into_owned();
    }
    out
}

fn walk(v: &mut Value, f: &mut dyn FnMut(&str, &mut Value)) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                f(k, x);
                walk(x, f);
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                f("", x);
                walk(x, f);
            }
        }
        _ => {}
    }
}

/// Replace model-authored text in one decoded message or delta object. Tool-call fragments are
/// left alone: they are the golden under test.
fn blank_model_text(o: &mut serde_json::Map<String, Value>) {
    for (k, label) in [
        ("content", "<MODEL_TEXT>"),
        ("reasoning", "<REASONING>"),
        ("reasoning_content", "<REASONING>"),
    ] {
        if let Some(Value::String(s)) = o.get_mut(k) {
            if !s.is_empty() {
                *s = label.into();
            }
        }
    }
}

/// Blank model text in the response chunks. Complete SSE events (`data: {json}`) are rewritten
/// in place; a non-stream JSON body is rewritten whole. Chunks that are not self contained
/// (split mid-event) are left for the scanner and the human reviewer.
fn scrub_response_text(v: &mut Value) {
    let Some(chunks) = v
        .pointer_mut("/response/chunks")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for c in chunks {
        let Some(s) = c.as_str() else { continue };
        let new = if let Ok(Value::Object(mut body)) = serde_json::from_str::<Value>(s) {
            if let Some(cs) = body.get_mut("choices").and_then(Value::as_array_mut) {
                for ch in cs {
                    for key in ["message", "delta"] {
                        if let Some(Value::Object(m)) = ch.get_mut(key) {
                            blank_model_text(m);
                        }
                    }
                }
            }
            Value::Object(body).to_string()
        } else {
            s.split_inclusive('\n')
                .map(|line| {
                    let Some(data) = line.strip_prefix("data: ") else {
                        return line.to_string();
                    };
                    match serde_json::from_str::<Value>(data.trim_end()) {
                        Ok(Value::Object(mut ev)) => {
                            if let Some(cs) = ev.get_mut("choices").and_then(Value::as_array_mut) {
                                for ch in cs {
                                    if let Some(Value::Object(d)) = ch.get_mut("delta") {
                                        blank_model_text(d);
                                    }
                                }
                            }
                            let tail = &line[data.trim_end().len() + 6..];
                            format!("data: {}{}", Value::Object(ev), tail)
                        }
                        _ => line.to_string(),
                    }
                })
                .collect()
        };
        *c = Value::String(new);
    }
}

/// Scrub a capture value BEFORE it is written: header values dropped, user content replaced by a
/// placeholder, tenant-style ids replaced, every string passed through [`scrub_text`].
pub fn scrub_value(v: &mut Value) {
    walk(v, &mut |k, x| {
        if sensitive_key(k) && x.is_string() {
            *x = Value::String("<ID>".into());
        }
    });
    // User and tool message content in the request body is never kept.
    if let Some(msgs) = v
        .pointer_mut("/request/body/messages")
        .and_then(Value::as_array_mut)
    {
        for m in msgs {
            let role = m
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if matches!(role.as_str(), "user" | "tool" | "system" | "assistant")
                && m.get("content").is_some_and(Value::is_string)
            {
                if let Some(c) = m.get_mut("content") {
                    *c = Value::String(format!("<{}_CONTENT>", role.to_ascii_uppercase()));
                }
            }
        }
    }
    if let Some(p) = v.pointer_mut("/request/body/prompt") {
        if p.is_string() {
            *p = Value::String("<USER_CONTENT>".into());
        }
    }
    scrub_response_text(v);
    walk(v, &mut |_, x| {
        if let Value::String(s) = x {
            *s = scrub_text(s);
        }
    });
}

/// Every refusal found in a candidate fixture. Empty means clean. Scans keys and string values.
pub fn scan(v: &Value) -> Vec<String> {
    let rules = compiled();
    let ph = placeholder();
    let mut found = Vec::new();
    let mut text = |where_: &str, s: &str, found: &mut Vec<String>| {
        for (class, re) in &rules {
            if re.is_match(s) {
                found.push(format!("{class} in {where_}"));
            }
        }
    };
    fn rec(
        v: &Value,
        path: &str,
        found: &mut Vec<String>,
        text: &mut dyn FnMut(&str, &str, &mut Vec<String>),
        ph: &Regex,
    ) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    let p = format!("{path}/{k}");
                    text(&p, k, found);
                    if sensitive_key(k) && !x.as_str().is_some_and(|s| ph.is_match(s)) {
                        found.push(format!("tenant-id key {k} without placeholder in {p}"));
                    }
                    rec(x, &p, found, text, ph);
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    rec(x, &format!("{path}/{i}"), found, text, ph);
                }
            }
            Value::String(s) => text(path, s, found),
            _ => {}
        }
    }
    rec(v, "", &mut found, &mut text, &ph);
    // Header lists carry names only.
    for side in ["/request/header_names", "/response/header_names"] {
        if let Some(a) = v.pointer(side).and_then(Value::as_array) {
            for h in a {
                match h.as_str() {
                    Some(n) if !n.contains(':') && !n.contains('=') && !n.contains(' ') => {}
                    _ => found.push(format!("header with a value in {side}")),
                }
            }
        }
    }
    // User and tool content in the request body must be a placeholder.
    if let Some(msgs) = v
        .pointer("/request/body/messages")
        .and_then(Value::as_array)
    {
        for (i, m) in msgs.iter().enumerate() {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            if matches!(role, "user" | "tool" | "system" | "assistant")
                && m.get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty() && !ph.is_match(s))
            {
                found.push(format!(
                    "unscrubbed {role} content in /request/body/messages/{i}"
                ));
            }
        }
    }
    if let Some(p) = v.pointer("/request/body/prompt").and_then(Value::as_str) {
        if !ph.is_match(p) {
            found.push("unscrubbed prompt in /request/body/prompt".into());
        }
    }
    found
}
