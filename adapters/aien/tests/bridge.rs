//! Drives the `aien-authority-bridge` binary over a real pipe: catalog, one authorized read, a
//! path escape that is denied with no read, a malformed line, and the approval round trip.
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

const TRACE: &str = "0123456789abcdef0123456789abcdef";

struct Bridge {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Bridge {
    fn start(ws: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_aien-authority-bridge"))
            .arg(ws)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn bridge");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn raw(&mut self, line: &str) -> Value {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut out = String::new();
        self.stdout.read_line(&mut out).unwrap();
        serde_json::from_str(&out).expect("one json line per request")
    }

    fn send(&mut self, v: Value) -> Value {
        self.raw(&v.to_string())
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(id: &str, cap: &str, args: Value) -> Value {
    json!({
        "kind": "capability_request", "request_id": id, "runtime": "aien",
        "capability": cap, "arguments": args,
        "tool": {"namespace": null, "name": cap},
        "mapping": {"table_version": "1", "rule_id": format!("passthrough:{cap}"), "passthrough": true}
    })
}

fn ctx() -> Value {
    json!({"trace_id": TRACE, "message_id": "m-1", "parent_id": null,
           "model": {"kind": "model", "id": "t"}, "exposure": null})
}

#[test]
fn catalog_read_escape_and_malformed_lines() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("a.txt"), "hello").unwrap();
    let outside = ws.path().parent().unwrap().join("bridge-outside.txt");
    std::fs::write(&outside, "SECRET").unwrap();
    let mut b = Bridge::start(ws.path());

    let cat = b.send(json!({"op": "catalog"}));
    assert_eq!(cat["ok"], true);
    assert_eq!(cat["catalog"]["runtime"], "aien");
    let names: Vec<&str> = cat["catalog"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"read_file") && names.contains(&"write_file"));

    let req = request("r-1", "read_file", json!({"path": "a.txt"}));
    let d = b.send(json!({"op": "decide", "request": req, "ctx": ctx()}));
    assert_eq!(d["decision"]["decision"], "authorized", "{d}");
    let res =
        b.send(json!({"op": "execute", "request": req, "decision": d["decision"], "ctx": ctx()}));
    assert_eq!(res["result"]["status"], "ok", "{res}");
    assert!(res["result"].to_string().contains("hello"));

    let esc = request("r-2", "read_file", json!({"path": "../bridge-outside.txt"}));
    let d = b.send(json!({"op": "decide", "request": esc, "ctx": ctx()}));
    assert_eq!(d["decision"]["decision"], "denied", "{d}");
    let res =
        b.send(json!({"op": "execute", "request": esc, "decision": d["decision"], "ctx": ctx()}));
    assert_ne!(res["result"]["status"], "ok", "{res}");
    assert!(!res.to_string().contains("SECRET"));

    for bad in [
        "not json",
        "[1]",
        r#"{"op":"nope"}"#,
        r#"{"op":"decide","request":{"x":1},"ctx":{}}"#,
        r#"{"op":"decide"}"#,
    ] {
        let r = b.raw(bad);
        assert_eq!(r["ok"], false, "{bad} -> {r}");
        assert!(r.get("decision").is_none() && r["error"].is_string());
    }
    let mut bad_ctx = ctx();
    bad_ctx["trace_id"] = json!("NOT-A-TRACE");
    let r = b.send(json!({"op": "decide", "request": request("r-3", "read_file", json!({"path": "a.txt"})), "ctx": bad_ctx}));
    assert_eq!(r["ok"], false);
    // still serving after the bad lines
    assert_eq!(b.send(json!({"op": "catalog"}))["ok"], true);
    let _ = std::fs::remove_file(outside);
}

#[test]
fn write_is_held_then_approved_once_and_executes_once() {
    let ws = tempfile::tempdir().unwrap();
    let mut b = Bridge::start(ws.path());
    let req = request(
        "w-1",
        "write_file",
        json!({"path": "n.txt", "content": "x"}),
    );
    // approving before the request was decided is refused by AIEN
    let early = b.send(json!({"op": "approve", "request": req}));
    assert_ne!(early["decision"]["decision"], "authorized", "{early}");
    let d = b.send(json!({"op": "decide", "request": req, "ctx": ctx()}));
    assert_eq!(d["decision"]["decision"], "requires_approval", "{d}");
    assert!(!ws.path().join("n.txt").exists());
    let a = b.send(json!({"op": "approve", "request": req}));
    assert_eq!(a["decision"]["decision"], "authorized", "{a}");
    let res =
        b.send(json!({"op": "execute", "request": req, "decision": a["decision"], "ctx": ctx()}));
    assert_eq!(res["result"]["status"], "ok", "{res}");
    assert_eq!(
        std::fs::read_to_string(ws.path().join("n.txt")).unwrap(),
        "x"
    );
    let again = b.send(json!({"op": "approve", "request": req}));
    assert_eq!(again["ok"], false, "{again}");
    assert!(again.get("decision").is_none());
    let twice =
        b.send(json!({"op": "execute", "request": req, "decision": a["decision"], "ctx": ctx()}));
    assert_ne!(twice["result"]["status"], "ok", "{twice}");
}
