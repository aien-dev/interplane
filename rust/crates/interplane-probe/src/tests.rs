use super::*;
use std::collections::HashMap;

fn chat(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}
fn calls(names: &[&str]) -> Value {
    let tc: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"id": format!("c{i}"), "type": "function", "function": {"name": n, "arguments": "{\"city\":\"Paris\"}"}}))
        .collect();
    json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": tc}}]})
}
fn pr(name: &str, v: Verdict) -> ProbeResult {
    ProbeResult {
        name: name.into(),
        verdict: v,
        detail: None,
        duration_ms: None,
        attempts: None,
        request_digest: None,
        response_digest: None,
        extensions: Map::new(),
    }
}

#[test]
fn chat_and_streaming_verdicts() {
    assert_eq!(eval_chat_basic(&chat("hello")).0, Verdict::Pass);
    assert_eq!(eval_chat_basic(&chat("  ")).0, Verdict::Fail);
    assert_eq!(eval_chat_basic(&json!({})).0, Verdict::Fail);
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\ndata: [DONE]\n";
    assert_eq!(assemble_sse(sse), (2, "Hello".to_string()));
    assert_eq!(eval_streaming(sse).0, Verdict::Pass);
    assert_eq!(eval_streaming("data: [DONE]").0, Verdict::Fail);
    assert_eq!(
        eval_streaming("data: {\"choices\":[{\"delta\":{}}]}").0,
        Verdict::Fail
    );
}
#[test]
fn tool_verdicts() {
    assert_eq!(
        eval_tools_native(&calls(&["get_weather"]), "get_weather").0,
        Verdict::Pass
    );
    assert_eq!(
        eval_tools_native(&calls(&["other"]), "get_weather").0,
        Verdict::Fail
    );
    assert_eq!(
        eval_tools_native(&chat("no call"), "get_weather").0,
        Verdict::Fail
    );
    let bad_args = json!({"choices": [{"message": {"tool_calls": [{"function": {"name": "get_weather", "arguments": "[1]"}}]}}]});
    assert_eq!(eval_tools_native(&bad_args, "get_weather").0, Verdict::Fail);
    assert_eq!(
        eval_parallel(&calls(&["read_file", "read_file"])).0,
        Verdict::Pass
    );
    assert_eq!(eval_parallel(&calls(&["get_weather"])).0, Verdict::Degraded);
    assert_eq!(eval_parallel(&chat("x")).0, Verdict::Fail);
    assert_eq!(
        eval_unknown_refusal(&chat("I cannot"), &["get_weather"]).0,
        Verdict::Pass
    );
    assert_eq!(
        eval_unknown_refusal(&calls(&["get_weather"]), &["get_weather"]).0,
        Verdict::Degraded
    );
    assert_eq!(
        eval_unknown_refusal(&calls(&["send_email"]), &["get_weather"]).0,
        Verdict::Fail
    );
}
#[test]
fn qwen35_text_probe_uses_the_lenshift_parser() {
    let ok = chat("<tool_call>\n<function=get_weather>\n<parameter=city>\nParis\n</parameter>\n</function>\n</tool_call>");
    assert_eq!(eval_text_qwen35(&ok, "get_weather").0, Verdict::Pass);
    assert_eq!(
        eval_text_qwen35(
            &chat("<tool_call>\n<function=get_weather>\n<parameter=city>\nPar"),
            "get_weather"
        )
        .0,
        Verdict::Fail
    );
    assert_eq!(
        eval_text_qwen35(&chat("It is sunny."), "get_weather").0,
        Verdict::Unsupported
    );
    assert_eq!(
        eval_text_qwen35(&chat(""), "get_weather").0,
        Verdict::Unsupported
    );
}
#[test]
fn replay_reasoning_json_context_models() {
    assert_eq!(
        eval_result_replay(&chat("It is 4217 degrees"), "4217").0,
        Verdict::Pass
    );
    assert_eq!(
        eval_result_replay(&chat("I do not know"), "4217").0,
        Verdict::Degraded
    );
    assert_eq!(eval_result_replay(&chat(""), "4217").0, Verdict::Fail);
    assert_eq!(
        eval_reasoning_channel(
            &json!({"choices": [{"message": {"content": "x", "reasoning_content": "hmm"}}]})
        )
        .0,
        Verdict::Pass
    );
    assert_eq!(
        eval_reasoning_channel(
            &json!({"choices": [{"message": {"content": "x", "reasoning": "hmm"}}]})
        )
        .0,
        Verdict::Pass
    );
    assert_eq!(
        eval_reasoning_channel(&chat("<think>a</think>b")).0,
        Verdict::Pass
    );
    assert_eq!(
        eval_reasoning_channel(&chat("plain")).0,
        Verdict::Unsupported
    );
    assert_eq!(
        eval_json_structured(&chat("{\"ok\": true}")).0,
        Verdict::Pass
    );
    assert_eq!(eval_json_structured(&chat("[1]")).0, Verdict::Fail);
    assert_eq!(eval_json_structured(&chat("sure!")).0, Verdict::Fail);
    assert_eq!(eval_context(&chat("7104"), "7104").0, Verdict::Pass);
    assert_eq!(eval_context(&chat("9999"), "7104").0, Verdict::Degraded);
    assert_eq!(eval_context(&chat(""), "7104").0, Verdict::Fail);
    assert_eq!(
        eval_models_list(&json!({"data": [{"id": "m"}]}), "m").0,
        Verdict::Pass
    );
    assert_eq!(
        eval_models_list(&json!({"data": [{"id": "x"}]}), "m").0,
        Verdict::Fail
    );
    assert_eq!(eval_models_list(&json!({}), "m").0, Verdict::Fail);
}
#[test]
fn http_status_classification() {
    let ok = |s: u16, b: &str| {
        classify(
            &Ok(HttpResponse {
                status: s,
                body: b.into(),
            }),
            true,
            true,
        )
    };
    assert!(matches!(ok(200, "{}"), Exchange::Body(_)));
    assert!(matches!(
        ok(400, "{}"),
        Exchange::Verdict((Verdict::Unsupported, _))
    ));
    assert!(matches!(
        ok(401, "{}"),
        Exchange::Verdict((Verdict::Fail, _))
    ));
    assert!(matches!(
        ok(500, "{}"),
        Exchange::Verdict((Verdict::Fail, _))
    ));
    assert!(matches!(
        ok(200, "<html>"),
        Exchange::Verdict((Verdict::Fail, _))
    ));
    assert!(matches!(
        classify(&Err("boom".into()), true, true),
        Exchange::Verdict((Verdict::Fail, _))
    ));
    // a basic chat probe is not a "feature": a 4xx is a failure, not "unsupported"
    assert!(matches!(
        classify(
            &Ok(HttpResponse {
                status: 404,
                body: String::new()
            }),
            false,
            true
        ),
        Exchange::Verdict((Verdict::Fail, _))
    ));
}
#[test]
fn profiles_follow_required_probes() {
    let all_pass = [
        "chat.basic",
        "models.list",
        "tools.native",
        "tools.result_replay",
        "tools.text_qwen35",
        "chat.streaming",
    ];
    let probes: Vec<_> = all_pass.iter().map(|n| pr(n, Verdict::Pass)).collect();
    assert!(compute_profiles(&probes)
        .iter()
        .all(|p| p.status == ProfileStatus::Compatible));

    let mut p = probes.clone();
    p.iter_mut()
        .find(|x| x.name == "tools.native")
        .unwrap()
        .verdict = Verdict::Fail;
    let prof = compute_profiles(&p);
    let st = |n: &str| prof.iter().find(|x| x.name == n).unwrap().status.clone();
    assert_eq!(st("lenshift.openai.1"), ProfileStatus::Incompatible);
    assert_eq!(st("lenshift.qwen35.1"), ProfileStatus::Compatible);

    let mut p = probes.clone();
    p.iter_mut()
        .find(|x| x.name == "chat.streaming")
        .unwrap()
        .verdict = Verdict::Skipped;
    assert_eq!(
        compute_profiles(&p)
            .iter()
            .find(|x| x.name == "relayline.tool_replay.1")
            .unwrap()
            .status,
        ProfileStatus::Untested
    );
    p.iter_mut()
        .find(|x| x.name == "chat.basic")
        .unwrap()
        .verdict = Verdict::Undetermined;
    assert_eq!(
        compute_profiles(&p)[0].status,
        ProfileStatus::Untested,
        "UNKNOWN is never treated as a pass"
    );
    assert_eq!(compute_profiles(&[])[0].status, ProfileStatus::Untested);
}
#[test]
fn urls_and_time() {
    assert_eq!(base_url("http://h:8000"), "http://h:8000/v1");
    assert_eq!(base_url("http://h:8000/v1/"), "http://h:8000/v1");
    assert_eq!(
        redact_endpoint("https://user:secret@host:8000/v1?key=abc#f"),
        "https://host:8000/v1"
    );
    assert_eq!(redact_endpoint("http://h"), "http://h");
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(1_767_225_600), "2026-01-01T00:00:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert!(needle_prompt(8, "7104").contains("The secret code word is 7104."));
    // lengths equal the Python probe's prompts for the same sizes
    assert_eq!(needle_prompt(8, "PELICAN-4410").len(), 25_010);
    assert_eq!(needle_prompt(32, "PELICAN-4410").len(), 98_654);
}

/// A canned server: answers by path and request shape. No network.
struct Canned {
    routes: HashMap<&'static str, (u16, String)>,
}
impl Transport for Canned {
    fn get(&self, path: &str) -> Result<HttpResponse, String> {
        let (s, b) = self.routes.get(path).cloned().unwrap_or((404, "{}".into()));
        Ok(HttpResponse { status: s, body: b })
    }
    fn post(&self, path: &str, body: &Value) -> Result<HttpResponse, String> {
        let key = if body.get("stream").is_some() {
            "stream"
        } else if body.get("tools").is_some() {
            "tools"
        } else {
            path
        };
        let (s, b) = self.routes.get(key).cloned().unwrap_or((500, "{}".into()));
        Ok(HttpResponse { status: s, body: b })
    }
}

#[test]
fn full_run_with_canned_endpoint_and_schema_valid_report() {
    let routes = HashMap::from([
        (
            "/chat/completions",
            (200, chat("hello 7104 {\"ok\": true}").to_string()),
        ),
        (
            "stream",
            (
                200,
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n"
                    .to_string(),
            ),
        ),
        (
            "tools",
            (200, calls(&["read_file", "read_file"]).to_string()),
        ),
        ("/models", (200, json!({"data": [{"id": "m"}]}).to_string())),
    ]);
    let cfg = ProbeConfig {
        endpoint: "http://user:pw@127.0.0.1:1/v1?token=abc".into(),
        model: "m".into(),
        backend: None,
        skip: vec!["context".into()],
        environment: detect_environment(),
    };
    let report = run_probes(&Canned { routes }, &cfg);
    let by = |n: &str| report.probes.iter().find(|p| p.name == n).unwrap();
    assert_eq!(by("chat.basic").verdict, Verdict::Pass);
    assert_eq!(by("chat.streaming").verdict, Verdict::Pass);
    assert_eq!(by("tools.native").verdict, Verdict::Pass);
    assert_eq!(by("tools.parallel").verdict, Verdict::Pass);
    assert_eq!(by("models.list").verdict, Verdict::Pass);
    assert_eq!(by("context.8k").verdict, Verdict::Skipped);
    assert_eq!(by("context.32k").verdict, Verdict::Skipped);
    assert_eq!(by("reasoning.channel").verdict, Verdict::Unsupported);
    assert_eq!(by("reasoning.disable").verdict, Verdict::Undetermined);
    assert!(
        by("chat.basic").request_digest.is_some() && by("chat.basic").response_digest.is_some()
    );
    assert_eq!(report.endpoint, "http://127.0.0.1:1/v1");
    assert!(!report_text(&report).contains("pw") && !report_text(&report).contains("abc"));
    assert_eq!(report.profiles[0].status, ProfileStatus::Compatible);

    // the report validates against spec/schemas/probe.schema.json
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../spec/schemas");
    let mut resources = vec![];
    for e in std::fs::read_dir(&dir).unwrap() {
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(e.unwrap().path()).unwrap()).unwrap();
        resources.push((
            v["$id"].as_str().unwrap().to_string(),
            jsonschema::Resource::from_contents(v).unwrap(),
        ));
    }
    let main: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("probe.schema.json")).unwrap())
            .unwrap();
    let v = jsonschema::options()
        .with_resources(resources.into_iter())
        .build(&main)
        .unwrap();
    let inst = serde_json::to_value(&report).unwrap();
    let errs: Vec<String> = v.iter_errors(&inst).map(|e| e.to_string()).collect();
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn dead_endpoint_fails_closed_never_passes() {
    struct Dead;
    impl Transport for Dead {
        fn get(&self, _: &str) -> Result<HttpResponse, String> {
            Err("connection refused".into())
        }
        fn post(&self, _: &str, _: &Value) -> Result<HttpResponse, String> {
            Err("connection refused".into())
        }
    }
    let cfg = ProbeConfig {
        endpoint: "http://x".into(),
        model: "m".into(),
        skip: vec!["context".into()],
        ..Default::default()
    };
    let report = run_probes(&Dead, &cfg);
    assert!(report.probes.iter().all(|p| matches!(
        p.verdict,
        Verdict::Fail | Verdict::Skipped | Verdict::Unsupported | Verdict::Undetermined
    )));
    assert!(report
        .profiles
        .iter()
        .all(|p| p.status != ProfileStatus::Compatible));
}

/// Records every POST body so tests can pin what the probe actually sends.
struct Recorder(std::sync::Mutex<Vec<Value>>);
impl Transport for Recorder {
    fn get(&self, _: &str) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 404,
            body: "{}".into(),
        })
    }
    fn post(&self, _: &str, body: &Value) -> Result<HttpResponse, String> {
        self.0.lock().unwrap().push(body.clone());
        Ok(HttpResponse {
            status: 500,
            body: "{}".into(),
        })
    }
}

/// Both reference probes send identical request bodies per probe id (spec/PROBE.md). The Python twin
/// is `test_chat_basic_request_body_is_pinned` in python/tests/test_probe_request_bodies.py.
#[test]
fn chat_basic_request_body_is_pinned() {
    let rec = Recorder(Default::default());
    let cfg = ProbeConfig {
        endpoint: "http://127.0.0.1:1/v1".into(),
        model: "m".into(),
        backend: None,
        skip: vec![],
        environment: detect_environment(),
    };
    let _ = run_probes(&rec, &cfg);
    let bodies = rec.0.lock().unwrap();
    assert_eq!(
        bodies[0],
        json!({"model": "m", "messages": [{"role": "user", "content": "Reply with the single word: ready"}],
               "max_tokens": 2048, "temperature": 0, "seed": 42})
    );
    // JCS digest pinned equal to the Python probe's request digest for the same body.
    assert_eq!(
        digest(&bodies[0]),
        "sha256:ed2eb0d61ca98c888bf63c7a884f2f568ee4bc54943f98acbe49db86ecdde923"
    );
}
