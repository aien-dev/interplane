//! Offline provider replay (issue #98). No network, no credentials.
use interplane_lenshift::replay::{check, replay, scan, scrub_text, scrub_value, Capture, FORMAT};
use serde_json::{json, Value};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dialects/replay")
}

fn collect(d: &std::path::Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(d).expect("dialects/replay present") {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(p);
        }
    }
}

/// Every fixture below `dialects/replay`, recursively.
fn load_all() -> Vec<(String, Value)> {
    let mut paths = Vec::new();
    collect(&dir(), &mut paths);
    let mut v: Vec<_> = paths
        .into_iter()
        .map(|p| {
            let t = std::fs::read_to_string(&p).unwrap();
            (p.display().to_string(), serde_json::from_str(&t).unwrap())
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn cap(name: &str) -> Capture {
    let (_, v) = load_all()
        .into_iter()
        .find(|(p, _)| p.ends_with(&format!("{name}.json")))
        .unwrap_or_else(|| panic!("fixture {name}"));
    serde_json::from_value(v).unwrap()
}

#[test]
fn every_fixture_is_clean_and_replays_to_its_golden() {
    let all = load_all();
    assert!(
        all.len() >= 11,
        "expected the replay fixtures, found {}",
        all.len()
    );
    for (path, v) in &all {
        assert_eq!(scan(v), Vec::<String>::new(), "{path} failed the scanner");
        let c: Capture =
            serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(c.format, FORMAT, "{path}");
        assert!(
            c.provenance.origin == "synthetic" || c.provenance.origin == "recorded",
            "{path} origin"
        );
        if let Err(e) = check(&c) {
            panic!("{e}");
        }
    }
}

#[test]
fn replay_is_deterministic() {
    for (_, v) in load_all() {
        let c: Capture = serde_json::from_value(v).unwrap();
        assert_eq!(replay(&c), replay(&c));
    }
}

#[test]
fn chunk_boundaries_do_not_change_the_outcome() {
    let c = cap("stream-tool-call-split-across-chunks");
    let whole: String = c.response.chunks.concat();
    for cut in [1, 7, whole.len() / 3, whole.len() - 1] {
        let mut d = c.clone();
        d.response.chunks = vec![whole[..cut].to_string(), whole[cut..].to_string()];
        assert_eq!(replay(&d), c.expect, "cut at {cut}");
    }
}

#[test]
fn tool_call_serialization_round_trips() {
    let c = cap("nonstream-tool-call-roundtrip");
    let first = replay(&c);
    let call = &first.tool_calls[0];
    // Re-serialize the parsed call as a provider would, replay it, and expect the same call.
    let body = json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[
        {"id":"call_rt","type":"function","function":{
            "name": call.name, "arguments": serde_json::to_string(&call.arguments).unwrap()}}]},
        "finish_reason":"tool_calls"}]});
    let mut d = c.clone();
    d.response.chunks = vec![body.to_string()];
    assert_eq!(replay(&d).tool_calls, first.tool_calls);
}

#[test]
fn drift_in_a_fixture_makes_the_runner_fail() {
    let c = cap("stream-tool-call-split-across-chunks");
    // Mutate the recorded bytes: the provider "renames" the arguments key.
    let mut d = c.clone();
    d.response.chunks = d
        .response
        .chunks
        .iter()
        .map(|s| s.replace("tool_calls", "tool_callz"))
        .collect();
    assert!(check(&d).is_err(), "renamed field must be detected");
    // Mutate the golden instead.
    let mut g = c.clone();
    g.expect.tool_calls[0].name = "other_tool".into();
    assert!(check(&g).is_err(), "changed golden must be detected");
    // Truncating the stream must be detected too.
    let mut t = c;
    let last = t.response.chunks.len() - 1;
    t.response.chunks[last].truncate(10);
    assert!(check(&t).is_err(), "truncation must be detected");
}

#[test]
fn error_statuses_never_yield_calls() {
    for (name, class) in [
        ("error-rate-limit-429", "rate_limited"),
        ("error-server-503", "server_error"),
    ] {
        let mut c = cap(name);
        assert_eq!(replay(&c).class, class);
        // Even a perfectly valid tool-call body under an error status is not parsed.
        c.response.chunks = vec![
            json!({"choices":[{"message":{"tool_calls":[{"id":"c","type":"function",
            "function":{"name":"read_file","arguments":"{}"}}]}}]})
            .to_string(),
        ];
        assert!(replay(&c).tool_calls.is_empty());
    }
}

#[test]
fn replay_output_carries_no_authority() {
    let c = cap("adversarial-authorization-claim-in-text");
    let out = replay(&c);
    assert!(out.text.contains("authorization: approved"));
    assert!(out.tool_calls.is_empty() && out.rejected.is_empty());
    // The outcome has a fixed closed shape: no decision, approval or grant field exists.
    let mut keys: Vec<String> = serde_json::to_value(&out)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, ["class", "partial", "rejected", "text", "tool_calls"]);
    // A fake call in prose stays text too.
    let f = replay(&cap("adversarial-fake-tool-call-in-prose"));
    assert!(f.tool_calls.is_empty() && f.text.contains("delete_all"));
}

/// A clean candidate to seed with one class of secret at a time.
fn candidate() -> Value {
    serde_json::to_value(cap("error-rate-limit-429")).unwrap()
}

#[test]
fn scanner_refuses_each_seeded_secret_class() {
    assert!(scan(&candidate()).is_empty());
    let seeds: &[(&str, &str, &str)] = &[
        ("bearer", "/response/chunks/0", "Bearer abcdEFGH12345678"),
        (
            "header-value",
            "/response/chunks/0",
            "Authorization: Basic dXNlcjpwYXNzd29yZA",
        ),
        (
            "cookie",
            "/response/chunks/0",
            "cookie: session=abcdef0123456789abcdef",
        ),
        (
            "api-key",
            "/response/chunks/0",
            "key sk-proj-AbCdEfGhIjKlMnOpQrSt",
        ),
        ("aws", "/response/chunks/0", "AKIAIOSFODNN7EXAMPLE"),
        (
            "hex",
            "/response/chunks/0",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        (
            "base64",
            "/response/chunks/0",
            "QWxhZGRpbjpvcGVuIHNlc2FtZVF1aWNrQnJvd25Gb3hKdW1wcw==",
        ),
        ("email", "/response/chunks/0", "mail drake@example.com now"),
        ("tenant-text", "/response/chunks/0", "workspace-ab12cd34ef"),
        ("query", "/request/path", "/v1/chat?api_key=abc123def"),
    ];
    for (label, ptr, seed) in seeds {
        let mut v = candidate();
        *v.pointer_mut(ptr).unwrap() = json!(seed);
        assert!(!scan(&v).is_empty(), "scanner accepted seeded {label}");
    }
    // Structured seeds.
    let mut v = candidate();
    v["request"]["header_names"] = json!(["Authorization: Bearer x1"]);
    assert!(!scan(&v).is_empty(), "header with value");
    let mut v = candidate();
    v["request"]["body"]["tenant_id"] = json!("t-9f8e7d6c");
    assert!(!scan(&v).is_empty(), "tenant id key");
    let mut v = candidate();
    v["request"]["body"]["messages"][0]["content"] = json!("please summarize my private notes");
    assert!(!scan(&v).is_empty(), "raw user content");
}

#[test]
fn scrubber_output_passes_the_scanner() {
    let mut v = candidate();
    v["response"]["chunks"][0] =
        json!("Bearer abcdEFGH12345678 and drake@example.com and sk-proj-AbCdEfGhIjKlMnOpQrSt");
    v["request"]["path"] = json!("/v1/chat?api_key=abc123def");
    v["request"]["body"]["tenant_id"] = json!("t-9f8e7d6c");
    v["request"]["body"]["messages"][0]["content"] = json!("my private text");
    assert!(!scan(&v).is_empty());
    scrub_value(&mut v);
    assert_eq!(scan(&v), Vec::<String>::new(), "after scrub: {v}");
    assert!(!v.to_string().contains("drake@example.com"));
    assert!(scrub_text("sk-proj-AbCdEfGhIjKlMnOpQrSt").contains("REDACTED"));
}

#[test]
fn authorization_word_in_model_text_is_not_a_secret() {
    // The adversarial fixture must stay admissible: prose is not a credential.
    assert!(scan(&json!({"c": "authorization: approved"})).is_empty());
}

#[test]
fn scanner_refuses_the_extra_classes() {
    let seeds: &[(&str, &str)] = &[
        ("ip", "server at 192.168.10.44 replied"),
        ("path", "opened /home/someone/project/secret.txt"),
        ("windows path", "C:\\Users\\someone\\file"),
        (
            "jwt",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcDEF123",
        ),
        ("pem", "-----BEGIN PRIVATE KEY-----"),
        ("stripe", "sk_live_abcdefgh12345678"),
        ("hf", "hf_AbCdEfGhIjKlMnOpQrSt"),
        ("cookie", "Cookie: sid=abc123"),
    ];
    for (label, seed) in seeds {
        let mut v = candidate();
        v["response"]["chunks"][0] = json!(seed);
        assert!(!scan(&v).is_empty(), "scanner accepted seeded {label}");
    }
    let mut v = candidate();
    v["request"]["body"]["prompt"] = json!("tell me a secret");
    assert!(!scan(&v).is_empty(), "raw prompt");
    let mut v = candidate();
    v["request"]["body"]["messages"] =
        json!([{"role":"system","content":"You are a private assistant for ACME."}]);
    assert!(!scan(&v).is_empty(), "raw system prompt");
}

#[test]
fn scrubber_blanks_model_text_but_keeps_tool_calls() {
    let mut v = candidate();
    v["request"]["body"]["messages"] = json!([
        {"role":"system","content":"private system prompt"},
        {"role":"assistant","content":"earlier private answer"},
        {"role":"user","content":"private question"}]);
    v["response"]["chunks"] = json!([
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"secret words\",\"reasoning\":\"private thought\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"a\\\"}\"}}]},\"finish_reason\":null}]}\n\n"]);
    scrub_value(&mut v);
    let s = v.to_string();
    for gone in ["private", "secret words"] {
        assert!(!s.contains(gone), "{gone} survived: {s}");
    }
    assert!(s.contains("<MODEL_TEXT>") && s.contains("<REASONING>"));
    assert!(
        s.contains("read_file") && s.contains("path"),
        "tool call must stay: {s}"
    );
    assert_eq!(scan(&v), Vec::<String>::new());
}

#[test]
fn recorded_fixture_keeps_no_real_model_text() {
    let (_, v) = load_all()
        .into_iter()
        .find(|(p, _)| p.contains("recorded-"))
        .unwrap();
    let s = v["response"]["chunks"].to_string();
    assert!(
        !s.contains("wants") && !s.contains("README.md. I need"),
        "reasoning text kept"
    );
    assert!(s.contains("<REASONING>"));
}

#[test]
fn fixture_discovery_is_recursive() {
    let root = std::env::temp_dir().join(format!("replay-rec-{}", std::process::id()));
    let deep = root.join("a").join("b");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("x.json"), "{}").unwrap();
    let mut out = Vec::new();
    collect(&root, &mut out);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(out.len(), 1, "depth-2 file must be found");
}
