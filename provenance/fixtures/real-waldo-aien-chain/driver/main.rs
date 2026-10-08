//! Real-run driver for the #76 provenance chain. Mirrors row 1 of adapters/aien/tests/compose_ledger.rs
//! (same adapter, same Pipeline, same approval desk) and retains the raw records the verifier reads.
//! Usage: real-run <aien-cli> <model-dir> <out-dir>
use interplane_adapter_aien::*;
use interplane_core::*;
use interplane_crossveil::{Pipeline, RuntimeAuthority};
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TRACE: &str = "real-run-03";
const PROMPT: &str = "Save the meeting summary to summary.txt.";
const MODEL_LABEL: &str = "toolcall-tiny";
const NOW: &str = "2026-10-06T00:00:00Z";
const CONTENT: &str = "Decisions: ship v0.3 on Friday.\nOwners: Ada (release), Lin (docs).\n";

fn hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn ts() -> String {
    let o = Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output().unwrap();
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (bin, model, out) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), PathBuf::from(&a[3]));
    let _ = std::fs::remove_dir_all(&out);
    let root = out.join("home");
    for d in ["ws", "compose", "state", "prov"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let root = std::fs::canonicalize(root).unwrap();
    let (sock, log) = (root.join("aien.sock"), out.join("daemon.log"));
    let made = Command::new(&bin)
        .args(["compose", "desk-key", "--create", "1"])
        .env("AIEN_COMPOSE_DIR", root.join("compose"))
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let txt = String::from_utf8_lossy(&made.stdout).to_string();
    let desk: Value = txt.lines().rev().find_map(|l| serde_json::from_str(l).ok()).expect("desk json");
    let desk_key_id = desk["desk_key_id"].as_str().unwrap().to_string();
    let mut child = Command::new(&bin)
        .arg("daemon")
        .env_remove("AIEN_GPU_BACKEND")
        .env_remove("AIEN_REQUIRE_BLACKWELL")
        .env_remove("AIEN_OMEGA_DIR")
        .env_remove("AIEN_GB10_QWEN3_DECLARED_ATTEMPT")
        .env("AIEN_MODEL_PATH", model.join("model.safetensors"))
        .env("AIEN_TOKENIZER_PATH", model.join("tokenizer.json"))
        .env("AIEN_REQUIRE_CHECKPOINT", "1")
        .env("AIEN_COMPOSE_DIR", root.join("compose"))
        .env("AIEN_RUNTIME_STATE_DIR", root.join("state"))
        .env("AIEN_PROVENANCE_DIR", root.join("prov"))
        .env("AIEN_RUNTIME_SOCK", &sock)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(std::fs::File::create(out.join("daemon.err")).unwrap())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if sock.exists() && text.contains("Warm-up:") {
            break;
        }
        if let Ok(Some(st)) = child.try_wait() {
            panic!("daemon exited {st}");
        }
        assert!(t0.elapsed() < Duration::from_secs(600), "no warm-up");
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_millis(500));
    // Daemon process identity (the bridge between the load log and the ledger's executor field).
    let pid = child.id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after = stat.rsplit_once(')').unwrap().1.split_whitespace().collect::<Vec<_>>();
    let start_ticks: u64 = after[19].parse().unwrap(); // field 22 overall
    let exe_sha = hex(&std::fs::read(&bin).unwrap());
    let ws = root.join("ws");
    let daemon_run = json!({"kind":"aien-daemon-run","pid":pid,"start_ticks":start_ticks,
        "socket":sock.to_str().unwrap(),"workspace":ws.to_str().unwrap(),"executable_sha256":exe_sha,
        "boot_id": std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim(),
        "note":"written by the run driver from /proc/<pid>/stat and the spawned child pid, not by the daemon"});

    let client = LedgerClient::new(&sock);
    let mut au = AienAuthority::new(ws.clone()).unwrap();
    au.set_clock(std::sync::Arc::new(|| 10));
    let shared = AienShared::new(au);
    let l = ComposeLedgerAuthority::new(shared, ws.clone(), &sock, root.join("compose/approval-desk.key")).unwrap();
    let mut rt = l.clone();
    let table = l.shared().with(|a| a.mapping_table(true));
    let mut p = Pipeline::new(DialectRegistry::with_defaults(), table, &mut rt, Limits::default(), RequestLedger::new());
    let body = "write the summary";
    p.register_input(serde_json::from_value(json!({
        "input_id":"in-user","content_kind":"user_request","trust":"user_supplied",
        "source":{"kind":"operator","id":"drake"},"origin":"operator:turn-0",
        "content_digest": format!("sha256:{}", hex(body.as_bytes())),"trace_id":TRACE,
        "parent_id":null,"derived_from":[]})).unwrap()).unwrap();
    // The model turn: AIEN's own generation path (StreamTurn) on the model this daemon loaded.
    let t_gen0 = ts();
    let messages = json!([{"role":"user","content":PROMPT}]);
    let (gen_text, _gen_tokens, gen_record) = stream_turn(&sock, &messages, 400);
    let t_gen1 = ts();
    let gen_id = gen_record.expect("daemon returned no generation_record (no record means no claim)");
    // The daemon's own record of the generation, exported with its own read command.
    let gview = client.recall(&[gen_id], None).unwrap();
    assert!(gview["missing"].as_array().unwrap().is_empty(), "{gview}");
    let gcited = gview["cited"].as_array().unwrap()[0].clone();
    wr_top(&out, "ledger-generation.json", &gcited);
    wr_top(&out, "model-turn.json", &json!({"dialect":"aien_legacy","model":MODEL_LABEL,"trace_id":TRACE,"turn":0,
        "input":gen_text,"generation_record_id":gen_id,"generation_record_source":"TurnFinished.generation_record",
        "stream_started":t_gen0,"stream_finished":t_gen1}));
    let t_req = ts();
    let o = p.run_turn("aien_legacy", MODEL_LABEL, &json!(gen_text), TRACE, 0);
    assert_eq!(o.results.len(), 1, "{o:?}");
    assert_eq!(o.results[0].status, ResultStatus::RequiresApproval, "{:?}", o.results[0]);
    let rid = o.intents[0].request_id.clone();
    let rid = rid.as_str();
    let intent = serde_json::to_value(&o.intents[0]).unwrap();
    let pending_result = serde_json::to_value(&o.results[0]).unwrap();
    let pa = p.pending_approval(TRACE, rid).expect("pending");
    let g = l.shared().with(|a| a.issue_approval(&pa.capability_request, 1_000)).unwrap();
    let (res, _) = l.continue_approval(&mut p, TRACE, rid, &g, 10, NOW, Some("drake")).expect("continued");
    assert_eq!(res.status, ResultStatus::Ok, "{res:?}");
    let t_done = ts();
    let final_result = serde_json::to_value(&res).unwrap();
    let rc = &final_result["data"]["receipt"];
    let (claim, ack) = (rc["handoff"]["replay_claim"].as_u64().unwrap(), rc["ack_record_id"].as_u64().unwrap());
    let ids: Vec<u64> = (claim..=ack).collect();
    let view = client.recall(&ids, None).unwrap();
    assert!(view["missing"].as_array().unwrap().is_empty(), "{view}");
    let cited = view["cited"].as_array().unwrap().clone();
    let by = |id: u64| cited.iter().find(|v| v["id"].as_u64() == Some(id)).unwrap().clone();
    let text_of = |v: &Value| serde_json::from_str::<Value>(v["text"].as_str().unwrap_or("")).unwrap_or(Value::Null);
    let committed = cited.iter().find(|v| text_of(v)["approved_submission"] == "committed").expect("committed").clone();
    let wr = |name: &str, v: &Value| {
        let d = out.join("records");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), serde_json::to_vec_pretty(v).unwrap()).unwrap();
    };
    wr("ledger-claim.json", &by(claim));
    wr("ledger-committed.json", &committed);
    wr("ledger-grant.json", &by(rc["grant_id"].as_u64().unwrap()));
    wr("ledger-intent.json", &by(rc["intent_id"].as_u64().unwrap()));
    wr("ledger-ack.json", &by(ack));
    wr("daemon-run.json", &daemon_run);
    wr("adapter-result.json", &final_result);
    wr("all-views-claim-to-ack.json", &view);
    let env = |n: u32, parent: Option<&str>, src: (&str, &str), dst: (&str, &str), time: &str, payload: Value| {
        let mut e = json!({"interplane_version":"0.1","message_id":format!("m-{n}"),"trace_id":TRACE,
            "timestamp":time,"source":{"kind":src.0,"id":src.1},"destination":{"kind":dst.0,"id":dst.1},"payload":payload});
        if let Some(pp) = parent { e["parent_id"] = json!(pp); }
        e
    };
    let trace = json!([
        env(1, None, ("model", MODEL_LABEL), ("runtime", "aien"), &t_req, intent),
        env(2, Some("m-1"), ("runtime", "aien"), ("model", MODEL_LABEL), &t_req, pending_result),
        env(3, Some("m-1"), ("runtime", "aien"), ("model", MODEL_LABEL), &t_done, final_result.clone()),
    ]);
    for e in trace.as_array().unwrap() {
        validate_envelope_value(e).unwrap_or_else(|c| panic!("envelope invalid {c:?}: {e}"));
    }
    wr("trace.json", &trace);
    wr("run-meta.json", &json!({"desk_key_id":desk_key_id,"trace_id":TRACE,"request_id":rid,
        "start_ms": t0.elapsed().as_millis() as u64,"claim":claim,"ack":ack}));
    let _ = Path::new(".");
    let _ = child.kill();
    let _ = child.wait();
    println!("OK claim={claim} ack={ack} pid={pid}");
}

fn wr_top(out: &Path, name: &str, v: &Value) {
    let d = out.join("records");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(name), serde_json::to_vec_pretty(v).unwrap()).unwrap();
}

fn load_log_digest(log: &Path, key: &str) -> String {
    let t = std::fs::read_to_string(log).unwrap();
    let i = t.find(key).expect("digest in load log") + key.len();
    t[i..].chars().take_while(|c| c.is_ascii_hexdigit()).collect()
}

/// One `StreamTurn` on the daemon socket (the path `aien-cli chat` uses): returns the finished text.
fn stream_turn(sock: &Path, messages: &Value, max_tokens: u64) -> (String, u64, Option<u64>) {
    let mut s = UnixStream::connect(sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(300))).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    let env = json!({"protocol_version":1,"request_id":now.as_millis() as u64,"operation_id":now.as_nanos() as u64,
        "operator_session":1,"command":{"StreamTurn":{"messages":messages,"max_tokens":max_tokens,"temperature":0.0}}});
    s.write_all(format!("{env}\n").as_bytes()).unwrap();
    for line in BufReader::new(&s).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if let Some(f) = v.get("TurnFinished") {
            return (f["text"].as_str().unwrap().to_string(), f["total_tokens"].as_u64().unwrap(), f["generation_record"].as_u64());
        }
        assert!(v.get("Error").is_none(), "{v}");
    }
    panic!("no TurnFinished");
}
