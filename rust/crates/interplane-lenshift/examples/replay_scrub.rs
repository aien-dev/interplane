//! Turn a qualification capture (`qualification/captures/**.json`, as written by the live probe
//! work) into a sanitized replay fixture. Scrubs first, scans second, and refuses to write if the
//! scan still finds anything. The golden `expect` is bootstrapped from the current parser and
//! MUST be reviewed by a human before commit.
//!
//! usage: replay_scrub <capture.json> <out.json> <provider> <source_commit> <captured_on>
use interplane_lenshift::replay::{replay, scan, scrub_value, Capture};
use serde_json::{json, Value};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 6 {
        eprintln!("usage: replay_scrub <capture.json> <out.json> <provider> <source_commit> <captured_on>");
        std::process::exit(2);
    }
    let src: Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("read")).expect("json");
    let stream = src["request"]["stream"].as_bool().unwrap_or(false);
    let body = src["response"].as_str().expect("response body is text");
    let chunks: Vec<String> = if stream {
        body.split_inclusive("\n\n").map(String::from).collect()
    } else {
        vec![body.to_string()]
    };
    let mut fx = json!({
        "format": "interplane-replay/1",
        "name": src["name"],
        "provider": a[3],
        "model": src["request"]["model"],
        "endpoint_kind": if stream { "chat_completions_stream" } else { "chat_completions" },
        "provenance": {"origin": "recorded", "source_commit": a[4], "captured_on": a[5],
            "note": format!("scrubbed from a qualification capture named {}", src["name"].as_str().unwrap_or("unnamed"))},
        "request": {"method": "POST", "path": src["endpoint"], "header_names": ["Content-Type"],
            "body": src["request"]},
        "response": {"status": 200, "header_names": ["Content-Type"], "chunks": chunks},
        "expect": {"class": "parsed", "text": "", "tool_calls": [], "rejected": [], "partial": false}
    });
    scrub_value(&mut fx);
    let cap: Capture = serde_json::from_value(fx.clone()).expect("shape");
    fx["expect"] = serde_json::to_value(replay(&cap)).unwrap();
    let findings = scan(&fx);
    if !findings.is_empty() {
        eprintln!(
            "refusing to write, still sensitive after scrub:\n{}",
            findings.join("\n")
        );
        std::process::exit(1);
    }
    std::fs::write(&a[2], serde_json::to_string_pretty(&fx).unwrap() + "\n").expect("write");
}
