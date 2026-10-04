//! Interplane Probe: measured capability qualification of an OpenAI-compatible endpoint.
//! Every verdict cites a request and response digest. Nothing is inferred from the model name.
//! The credential (`INTERPLANE_PROBE_API_KEY`) is used only in a request header and never stored.
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use interplane_core::{
    canonicalize, digest, digest_bytes, Environment, ProbeReport, ProbeReportKind, ProbeResult,
    ProfileResult, ProfileStatus, Verdict,
};
use interplane_lenshift::{Dialect, ParseContext};
use serde_json::{json, Map, Value};

pub const PROBE_VERSION: &str = "0.1.0";

/// A raw HTTP answer.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// The only thing the probe needs from the network. Tests supply canned answers.
pub trait Transport {
    fn get(&self, path: &str) -> Result<HttpResponse, String>;
    fn post(&self, path: &str, body: &Value) -> Result<HttpResponse, String>;
}

/// `ureq`-backed transport.
pub struct UreqTransport {
    base: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new(endpoint: &str, api_key: Option<String>, timeout_secs: u64) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build();
        Self {
            base: base_url(endpoint),
            api_key,
            agent,
        }
    }
    fn finish(&self, r: Result<ureq::Response, ureq::Error>) -> Result<HttpResponse, String> {
        match r {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.into_string().map_err(|e| self.scrub(e.to_string()))?;
                Ok(HttpResponse { status, body })
            }
            Err(ureq::Error::Status(status, resp)) => Ok(HttpResponse {
                status,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(self.scrub(e.to_string())),
        }
    }
    fn scrub(&self, s: String) -> String {
        match &self.api_key {
            Some(k) if !k.is_empty() => s.replace(k.as_str(), "[redacted]"),
            _ => s,
        }
    }
}

impl Transport for UreqTransport {
    fn get(&self, path: &str) -> Result<HttpResponse, String> {
        let mut req = self.agent.get(&format!("{}{}", self.base, path));
        if let Some(k) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        self.finish(req.call())
    }
    fn post(&self, path: &str, body: &Value) -> Result<HttpResponse, String> {
        let mut req = self.agent.post(&format!("{}{}", self.base, path));
        if let Some(k) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        self.finish(req.send_json(body.clone()))
    }
}

/// The API base: the endpoint without a trailing slash, with `/v1` appended unless present.
pub fn base_url(endpoint: &str) -> String {
    let e = endpoint.trim_end_matches('/');
    if e.ends_with("/v1") {
        e.to_string()
    } else {
        format!("{e}/v1")
    }
}

/// Redacted endpoint: no credentials, no query string, no fragment.
pub fn redact_endpoint(url: &str) -> String {
    let (scheme, rest) = url
        .split_once("://")
        .map(|(s, r)| (format!("{s}://"), r))
        .unwrap_or((String::new(), url));
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let rest = match rest.split_once('/') {
        Some((auth, path)) => format!("{}/{}", auth.rsplit('@').next().unwrap_or(auth), path),
        None => rest.rsplit('@').next().unwrap_or(rest).to_string(),
    };
    format!("{scheme}{}", rest.trim_end_matches('/'))
}

/// Settings for one run.
#[derive(Debug, Clone, Default)]
pub struct ProbeConfig {
    pub endpoint: String,
    pub model: String,
    pub backend: Option<String>,
    /// Probe names or prefixes to skip (`context` skips every `context.*`).
    pub skip: Vec<String>,
    pub environment: Environment,
}

// ---------- pure evaluators (unit-tested with canned responses) ----------

/// `choices[0].message`.
fn message(v: &Value) -> Option<&Value> {
    v.pointer("/choices/0/message")
}
fn content(v: &Value) -> String {
    message(v)
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
fn tool_calls(v: &Value) -> Vec<&Value> {
    message(v)
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}
fn call_name(c: &Value) -> &str {
    c.pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or("")
}
fn call_args_object(c: &Value) -> bool {
    match c.pointer("/function/arguments") {
        Some(Value::String(s)) => matches!(serde_json::from_str::<Value>(s), Ok(Value::Object(_))),
        Some(Value::Object(_)) => true,
        _ => false,
    }
}
/// Reasoning text from `reasoning_content`, `reasoning`, or a `<think>` block in content.
pub fn reasoning_text(v: &Value) -> Option<String> {
    let m = message(v)?;
    for k in ["reasoning_content", "reasoning"] {
        if let Some(s) = m
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            return Some(s.to_string());
        }
    }
    let c = content(v);
    c.contains("<think>").then_some(c)
}

type Eval = (Verdict, String);
fn ev(v: Verdict, d: &str) -> Eval {
    (v, d.to_string())
}

pub fn eval_chat_basic(v: &Value) -> Eval {
    if content(v).trim().is_empty() {
        if reasoning_text(v).is_some() {
            ev(
                Verdict::Fail,
                "empty content; reasoning channel consumed the completion budget",
            )
        } else {
            ev(Verdict::Fail, "empty content")
        }
    } else {
        ev(Verdict::Pass, "non-empty content")
    }
}
/// Assemble an SSE body into text.
pub fn assemble_sse(body: &str) -> (usize, String) {
    let (mut chunks, mut text) = (0, String::new());
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(data) {
            chunks += 1;
            if let Some(s) = v
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                text.push_str(s);
            }
        }
    }
    (chunks, text)
}
pub fn eval_streaming(body: &str) -> Eval {
    let (chunks, text) = assemble_sse(body);
    if chunks == 0 {
        ev(Verdict::Fail, "no SSE chunks")
    } else if text.trim().is_empty() {
        ev(Verdict::Fail, "chunks assembled to empty content")
    } else {
        ev(
            Verdict::Pass,
            &format!("{chunks} chunks assembled to non-empty content"),
        )
    }
}
pub fn eval_tools_native(v: &Value, want: &str) -> Eval {
    let calls = tool_calls(v);
    match calls.first() {
        None => ev(Verdict::Fail, "no tool_calls in response"),
        Some(c) if call_name(c) != want => ev(
            Verdict::Fail,
            &format!("called {:?}, wanted {want:?}", call_name(c)),
        ),
        Some(c) if !call_args_object(c) => ev(Verdict::Fail, "arguments are not a JSON object"),
        Some(_) => ev(Verdict::Pass, "tool call with object arguments"),
    }
}
pub fn eval_text_qwen35(v: &Value, want: &str) -> Eval {
    let turn = interplane_lenshift::qwen35::Qwen35.parse(
        &Value::String(content(v)),
        &ParseContext {
            trace_id: "probe".into(),
            turn: 0,
            model: "probe".into(),
        },
    );
    if turn.partial || !turn.rejected.is_empty() {
        ev(
            Verdict::Fail,
            "tool call markup present but malformed or truncated",
        )
    } else if turn.intents.iter().any(|i| i.tool.name == want) {
        ev(Verdict::Pass, "parseable <tool_call><function=...> block")
    } else if !tool_calls(v).is_empty() {
        ev(
            Verdict::Degraded,
            "backend parsed the text form into native tool_calls itself (content empty)",
        )
    } else if content(v).trim().is_empty() {
        ev(
            Verdict::Unsupported,
            "empty content and no tool_calls: the backend parser consumed or suppressed the text-form call (compare a raw-completion capture)",
        )
    } else if content(v).contains("<tool_call>") {
        ev(
            Verdict::Degraded,
            "tool_call markup present but not parseable as qwen35",
        )
    } else {
        ev(Verdict::Unsupported, "no <tool_call> markup in content")
    }
}
pub fn eval_parallel(v: &Value) -> Eval {
    match tool_calls(v).len() {
        0 => ev(Verdict::Fail, "no tool_calls"),
        1 => ev(Verdict::Degraded, "only one tool call in the turn"),
        n => ev(Verdict::Pass, &format!("{n} tool calls in one turn")),
    }
}
pub fn eval_result_replay(v: &Value, needle: &str) -> Eval {
    let c = content(v);
    if c.contains(needle) {
        ev(Verdict::Pass, "final answer uses the tool result")
    } else if c.trim().is_empty() {
        ev(Verdict::Fail, "no final answer")
    } else {
        ev(Verdict::Degraded, "answered without using the tool result")
    }
}
pub fn eval_unknown_refusal(v: &Value, provided: &[&str]) -> Eval {
    let calls = tool_calls(v);
    if calls.is_empty() {
        ev(Verdict::Pass, "did not call a tool that was not provided")
    } else if calls.iter().all(|c| provided.contains(&call_name(c))) {
        ev(
            Verdict::Degraded,
            "called a provided tool instead of declining",
        )
    } else {
        ev(Verdict::Fail, "called a nonexistent tool")
    }
}
pub fn eval_reasoning_channel(v: &Value) -> Eval {
    if reasoning_text(v).is_some() {
        ev(Verdict::Pass, "reasoning channel present")
    } else {
        ev(
            Verdict::Unsupported,
            "no reasoning_content, reasoning or <think> seen",
        )
    }
}
pub fn eval_json_structured(v: &Value) -> Eval {
    match serde_json::from_str::<Value>(content(v).trim()) {
        Ok(Value::Object(_)) => ev(Verdict::Pass, "valid JSON object"),
        Ok(_) => ev(Verdict::Fail, "valid JSON but not an object"),
        Err(_) => ev(Verdict::Fail, "content is not valid JSON"),
    }
}
pub fn eval_context(v: &Value, needle: &str) -> Eval {
    let c = content(v);
    if c.contains(needle) {
        ev(Verdict::Pass, "needle found")
    } else if c.trim().is_empty() {
        ev(Verdict::Fail, "empty answer")
    } else {
        ev(Verdict::Degraded, "answered but the needle was wrong")
    }
}
pub fn eval_models_list(v: &Value, model: &str) -> Eval {
    let ids: Vec<&str> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    if ids.contains(&model) {
        ev(Verdict::Pass, "model listed")
    } else if ids.is_empty() && v.get("data").is_none() {
        ev(Verdict::Fail, "response has no data array")
    } else {
        ev(Verdict::Fail, "model not listed")
    }
}

/// Profiles and their required probes (PROBE.md).
pub const PROFILES: [(&str, &[&str]); 4] = [
    ("interplane.core.0.1", &["chat.basic", "models.list"]),
    (
        "lenshift.openai.1",
        &["tools.native", "tools.result_replay"],
    ),
    (
        "lenshift.qwen35.1",
        &["tools.text_qwen35", "tools.result_replay"],
    ),
    (
        "relayline.tool_replay.1",
        &["tools.result_replay", "chat.streaming"],
    ),
];

/// compatible only when every required probe PASSed; incompatible when any ran and did not PASS
/// (UNKNOWN and SKIPPED are not failures); untested otherwise.
pub fn compute_profiles(probes: &[ProbeResult]) -> Vec<ProfileResult> {
    PROFILES
        .iter()
        .map(|(name, req)| {
            let verdicts: Vec<Option<&Verdict>> = req
                .iter()
                .map(|r| probes.iter().find(|p| p.name == *r).map(|p| &p.verdict))
                .collect();
            let status = if verdicts.iter().all(|v| matches!(v, Some(Verdict::Pass))) {
                ProfileStatus::Compatible
            } else if verdicts.iter().any(|v| {
                matches!(
                    v,
                    Some(Verdict::Fail | Verdict::Degraded | Verdict::Unsupported)
                )
            }) {
                ProfileStatus::Incompatible
            } else {
                ProfileStatus::Untested
            };
            ProfileResult {
                name: (*name).into(),
                status,
                required_probes: req.iter().map(|s| s.to_string()).collect(),
                extensions: Map::new(),
            }
        })
        .collect()
}

// ---------- running ----------

/// How an HTTP exchange maps to a verdict before any content evaluation.
#[derive(Debug)]
enum Exchange {
    Body(Value),
    Text(String),
    /// Verdict decided by the transport/status alone.
    Verdict(Eval),
}

fn classify(r: &Result<HttpResponse, String>, feature_probe: bool, want_json: bool) -> Exchange {
    match r {
        Err(e) => Exchange::Verdict(ev(Verdict::Fail, &format!("transport error: {e}"))),
        Ok(h) if h.status == 401 || h.status == 403 => Exchange::Verdict(ev(
            Verdict::Fail,
            &format!("HTTP {}: authentication required", h.status),
        )),
        Ok(h) if (400..500).contains(&h.status) && feature_probe => Exchange::Verdict(ev(
            Verdict::Unsupported,
            &format!("HTTP {}: endpoint rejected the request", h.status),
        )),
        Ok(h) if h.status >= 400 => {
            Exchange::Verdict(ev(Verdict::Fail, &format!("HTTP {}", h.status)))
        }
        Ok(h) if want_json => match serde_json::from_str::<Value>(&h.body) {
            Ok(v) => Exchange::Body(v),
            Err(_) => Exchange::Verdict(ev(Verdict::Fail, "response is not JSON")),
        },
        Ok(h) => Exchange::Text(h.body.clone()),
    }
}

struct Runner<'a> {
    t: &'a dyn Transport,
    cfg: &'a ProbeConfig,
    results: Vec<ProbeResult>,
}

fn weather_tool() -> Value {
    json!({"type": "function", "function": {"name": "get_weather", "description": "Get the current weather for a city.",
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}})
}

impl Runner<'_> {
    fn skipped(&self, name: &str) -> bool {
        self.cfg
            .skip
            .iter()
            .any(|s| name == s || name.starts_with(&format!("{s}.")))
    }
    fn chat(&self, messages: Value) -> Value {
        json!({"model": self.cfg.model, "messages": messages, "temperature": 0, "max_tokens": 2048})
    }
    fn record(&mut self, name: &str, verdict: Eval, req: &Value, resp: Option<&str>, ms: u64) {
        self.results.push(ProbeResult {
            name: name.into(),
            verdict: verdict.0,
            detail: Some(verdict.1.chars().take(4096).collect()),
            duration_ms: Some(ms),
            attempts: Some(1),
            request_digest: Some(digest(req)),
            response_digest: resp.map(|r| match serde_json::from_str::<Value>(r) {
                Ok(v) => digest(&v),
                Err(_) => digest_bytes(r.as_bytes()),
            }),
            extensions: Map::new(),
        });
    }
    fn skip(&mut self, name: &str) {
        self.results.push(ProbeResult {
            name: name.into(),
            verdict: Verdict::Skipped,
            detail: Some("skipped by request".into()),
            duration_ms: None,
            attempts: None,
            request_digest: None,
            response_digest: None,
            extensions: Map::new(),
        });
    }
    /// POST a chat body and evaluate the JSON answer. Returns the answer when the call succeeded.
    fn post_eval(
        &mut self,
        name: &str,
        body: Value,
        feature: bool,
        f: impl Fn(&Value) -> Eval,
    ) -> Option<Value> {
        if self.skipped(name) {
            self.skip(name);
            return None;
        }
        let t0 = Instant::now();
        let r = self.t.post("/chat/completions", &body);
        let ms = t0.elapsed().as_millis() as u64;
        let raw = r.as_ref().ok().map(|h| h.body.clone());
        match classify(&r, feature, true) {
            Exchange::Body(v) => {
                let verdict = f(&v);
                self.record(name, verdict, &body, raw.as_deref(), ms);
                Some(v)
            }
            Exchange::Verdict(v) => {
                self.record(name, v, &body, raw.as_deref(), ms);
                None
            }
            Exchange::Text(_) => None,
        }
    }

    fn run(&mut self) {
        let user = |s: &str| json!([{"role": "user", "content": s}]);
        self.post_eval(
            "chat.basic",
            self.chat(user("Reply with one short sentence about the sea.")),
            false,
            eval_chat_basic,
        );

        // streaming
        let name = "chat.streaming";
        if self.skipped(name) {
            self.skip(name);
        } else {
            let mut body = self.chat(user("Count from one to five."));
            body["stream"] = json!(true);
            let t0 = Instant::now();
            let r = self.t.post("/chat/completions", &body);
            let ms = t0.elapsed().as_millis() as u64;
            let raw = r.as_ref().ok().map(|h| h.body.clone());
            let verdict = match classify(&r, true, false) {
                Exchange::Text(s) => eval_streaming(&s),
                Exchange::Verdict(v) => v,
                Exchange::Body(_) => ev(Verdict::Fail, "unexpected body"),
            };
            self.record(name, verdict, &body, raw.as_deref(), ms);
        }

        let mut native = self.chat(user("What is the weather in Paris? Use the tool."));
        native["tools"] = json!([weather_tool()]);
        self.post_eval("tools.native", native, true, |v| {
            eval_tools_native(v, "get_weather")
        });

        let sys = "You can call functions. To call one, reply with exactly this format and nothing else:\n<tool_call>\n<function=get_weather>\n<parameter=city>\nCITY\n</parameter>\n</function>\n</tool_call>\nReplace CITY with the city name.";
        let text_body = self.chat(json!([{"role": "system", "content": sys}, {"role": "user", "content": "What is the weather in Paris?"}]));
        self.post_eval("tools.text_qwen35", text_body, false, |v| {
            eval_text_qwen35(v, "get_weather")
        });

        let mut par = self.chat(user("Get the weather in Paris and in Tokyo. Call the tool for both cities in the same turn."));
        par["tools"] = json!([weather_tool()]);
        self.post_eval("tools.parallel", par, true, eval_parallel);

        let mut replay = self.chat(json!([
            {"role": "user", "content": "What is the weather in Paris?"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "call_probe_1", "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}}]},
            {"role": "tool", "tool_call_id": "call_probe_1", "content": "{\"city\":\"Paris\",\"temperature_c\":4217}"}
        ]));
        replay["tools"] = json!([weather_tool()]);
        self.post_eval("tools.result_replay", replay, true, |v| {
            eval_result_replay(v, "4217")
        });

        let mut unk = self.chat(user(
            "Send an email to bob@example.com saying hello. Use the send_email tool.",
        ));
        unk["tools"] = json!([weather_tool()]);
        self.post_eval("tools.unknown_refusal", unk, true, |v| {
            eval_unknown_refusal(v, &["get_weather"])
        });

        let chan = self.post_eval(
            "reasoning.channel",
            self.chat(user("What is 17 times 23? Think step by step.")),
            true,
            eval_reasoning_channel,
        );
        self.reasoning_disable(chan.as_ref().is_some_and(|v| reasoning_text(v).is_some()));

        let mut js = self.chat(user("Return a JSON object with the key ok set to true."));
        js["response_format"] = json!({"type": "json_object"});
        self.post_eval("json.structured", js, true, eval_json_structured);

        for (name, kib) in [
            ("context.8k", 8usize),
            ("context.16k", 16),
            ("context.32k", 32),
        ] {
            let needle = format!("{}", 7000 + kib * 13);
            let body = self.chat(user(&needle_prompt(kib, &needle)));
            self.post_eval(name, body, false, |v| eval_context(v, &needle));
        }

        self.models_list();
    }

    fn reasoning_disable(&mut self, baseline_has_reasoning: bool) {
        let name = "reasoning.disable";
        if self.skipped(name) {
            self.skip(name);
            return;
        }
        let base = self
            .chat(json!([{"role": "user", "content": "What is 17 times 23? Think step by step."}]));
        let mechanisms: [(&str, Value); 3] = [
            (
                "chat_template_kwargs.enable_thinking=false",
                json!({"chat_template_kwargs": {"enable_thinking": false}}),
            ),
            ("think=false", json!({"think": false})),
            ("reasoning_effort=none", json!({"reasoning_effort": "none"})),
        ];
        let t0 = Instant::now();
        let (mut last_body, mut last_raw) = (base.clone(), None);
        let mut accepted = 0;
        let mut verdict = None;
        for (label, extra) in mechanisms {
            let mut body = base.clone();
            for (k, v) in extra.as_object().cloned().unwrap_or_default() {
                body[k] = v;
            }
            let r = self.t.post("/chat/completions", &body);
            last_body = body;
            last_raw = r.as_ref().ok().map(|h| h.body.clone());
            match classify(&r, true, true) {
                Exchange::Body(v) => {
                    accepted += 1;
                    if reasoning_text(&v).is_none() {
                        verdict = Some(if baseline_has_reasoning {
                            ev(Verdict::Pass, &format!("reasoning absent with {label}"))
                        } else {
                            ev(Verdict::Undetermined, "no reasoning channel without the switch either, so disabling is not demonstrated")
                        });
                        break;
                    }
                }
                Exchange::Verdict((Verdict::Fail, d)) => {
                    verdict = Some((Verdict::Fail, d));
                    break;
                }
                _ => {}
            }
        }
        let verdict = verdict.unwrap_or_else(|| {
            if accepted == 0 {
                ev(Verdict::Unsupported, "no disable mechanism was accepted")
            } else {
                ev(
                    Verdict::Fail,
                    "reasoning still present with every accepted disable mechanism",
                )
            }
        });
        let ms = t0.elapsed().as_millis() as u64;
        self.record(name, verdict, &last_body, last_raw.as_deref(), ms);
    }

    fn models_list(&mut self) {
        let name = "models.list";
        if self.skipped(name) {
            self.skip(name);
            return;
        }
        let t0 = Instant::now();
        let r = self.t.get("/models");
        let ms = t0.elapsed().as_millis() as u64;
        let raw = r.as_ref().ok().map(|h| h.body.clone());
        let verdict = match classify(&r, true, true) {
            Exchange::Body(v) => eval_models_list(&v, &self.cfg.model),
            Exchange::Verdict(v) => v,
            Exchange::Text(_) => ev(Verdict::Fail, "unexpected body"),
        };
        self.record(
            name,
            verdict,
            &json!({"GET": "/models"}),
            raw.as_deref(),
            ms,
        );
    }
}

/// A prompt of roughly `kib` thousand tokens with the needle in the middle.
pub fn needle_prompt(kib: usize, needle: &str) -> String {
    let filler = "The quick brown fox jumps over the lazy dog. ";
    let n = kib * 1024 * 4 / filler.len();
    let half = filler.repeat(n / 2);
    format!("{half}The secret code is {needle}. {half}\nWhat is the secret code? Answer with the number only.")
}

/// RFC 3339 UTC timestamp for `secs` since the epoch.
pub fn rfc3339(secs: u64) -> String {
    let (days, rem) = (secs / 86400, secs % 86400);
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn now() -> String {
    rfc3339(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// Hardware and OS from /proc (Linux), falling back to the compile-time architecture.
pub fn detect_environment() -> Environment {
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .ok()
            .map(|s| s.trim().trim_end_matches('\0').to_string())
            .filter(|s| !s.is_empty())
    };
    let cpu = read("/proc/device-tree/model").or_else(|| {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()?
            .lines()
            .find_map(|l| {
                l.strip_prefix("model name")
                    .and_then(|r| r.split_once(':'))
                    .map(|(_, v)| v.trim().to_string())
            })
    });
    let hardware = Some(match cpu {
        Some(c) => format!("{c} ({})", std::env::consts::ARCH),
        None => std::env::consts::ARCH.to_string(),
    });
    let os = match (
        read("/proc/sys/kernel/ostype"),
        read("/proc/sys/kernel/osrelease"),
    ) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        _ => Some(std::env::consts::OS.to_string()),
    };
    Environment {
        hardware,
        os,
        endpoint_type: Some("openai_compatible".into()),
        interplane_version: Some(interplane_core::PROTOCOL_VERSION.into()),
        fixture_revision: std::env::var("INTERPLANE_FIXTURE_REVISION").ok(),
        model_revision: None,
        extensions: Map::new(),
    }
}

/// Run every probe and build the report.
pub fn run_probes(t: &dyn Transport, cfg: &ProbeConfig) -> ProbeReport {
    let started = now();
    let mut r = Runner {
        t,
        cfg,
        results: vec![],
    };
    r.run();
    let probes = r.results;
    let profiles = compute_profiles(&probes);
    ProbeReport {
        kind: ProbeReportKind,
        probe_version: PROBE_VERSION.into(),
        endpoint: redact_endpoint(&cfg.endpoint),
        backend: cfg.backend.clone(),
        backend_version: None,
        model: cfg.model.clone(),
        model_digest: Some(digest(
            &json!({"model": cfg.model, "checkpoint": cfg.environment.model_revision}),
        )),
        started_at: started,
        finished_at: Some(now()),
        probes,
        profiles,
        environment: Some(cfg.environment.clone()),
        extensions: Map::new(),
    }
}

/// Canonical text of a report (for stable files).
pub fn report_text(r: &ProbeReport) -> String {
    canonicalize(&serde_json::to_value(r).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests;
