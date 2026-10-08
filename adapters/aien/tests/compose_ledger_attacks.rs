//! Attacks on the single write route of the compose-ledger cut (sovereign-core #249 item 1,
//! interplane #77): the old client-minted authorization route can produce no write, there is no
//! fallback write when the daemon refuses or is gone, the effect names the daemon's grant, and
//! the write itself cannot be redirected outside the workspace.
//!
//! Same harness as `compose_ledger.rs` (helpers copied verbatim): needs `AIEN_BIN`, rows are
//! `#[ignore]` (NOT_RUN) unless run with `--ignored`.
#![allow(dead_code)]
use interplane_adapter_aien::*;
use interplane_core::*;
use interplane_crossveil::Pipeline;
use interplane_lenshift::DialectRegistry;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const NOW: &str = "2026-10-06T00:00:00Z";
const TRACE: &str = "ledger";
const EXPECTED: &str = "Decisions: ship v0.3 on Friday.\nOwners: Ada (release), Lin (docs).\n";
const MODEL_SNAPSHOT: &str = ".cache/huggingface/hub/models--unsloth--Llama-3.2-1B-Instruct/snapshots/5a8abab4a5d6f164389b1079fb721cfab8d7126c";

/// One daemon at a time (each loads the model).
static SERIAL: Mutex<()> = Mutex::new(());

fn hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

struct Daemon {
    child: Child,
    root: PathBuf,
    sock: PathBuf,
    desk_key_id: String,
    log: PathBuf,
    started_ms: u128,
    _keep: Option<tempfile::TempDir>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    fn client(&self) -> LedgerClient {
        LedgerClient::new(&self.sock)
    }
    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }
    fn desk(&self) -> PathBuf {
        self.root.join("compose").join("approval-desk.key")
    }
}

/// Start `AIEN_BIN daemon` on CPU in a fresh home. `None` (NOT_RUN, printed) when `AIEN_BIN` is
/// unset. Panics with the log path when the daemon does not come up within 600 s.
fn daemon(row: &str) -> Option<Daemon> {
    let Ok(bin) = std::env::var("AIEN_BIN") else {
        panic!("NOT_RUN {row}: AIEN_BIN is unset (needs an aien-cli daemon binary)");
    };
    let model = std::env::var("AIEN_LEDGER_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(&std::env::var("HOME").unwrap_or_default()).join(MODEL_SNAPSHOT)
        });
    let (keep, root) = match std::env::var("LEDGER_OUT") {
        Ok(d) => {
            let p = Path::new(&d).join(row);
            let _ = std::fs::remove_dir_all(&p);
            (None, p)
        }
        Err(_) => {
            let t = tempfile::tempdir().unwrap();
            let p = t.path().to_path_buf();
            (Some(t), p)
        }
    };
    for d in ["ws", "compose", "state", "prov"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let root = std::fs::canonicalize(root).unwrap();
    let (sock, log) = (root.join("aien.sock"), root.join("daemon.log"));
    // The approval desk key, made by the CLI step in the compose home (outside the workspace).
    let made = Command::new(&bin)
        .args(["compose", "desk-key", "--create", "1"])
        .env("AIEN_COMPOSE_DIR", root.join("compose"))
        .output()
        .expect("aien compose desk-key");
    assert!(made.status.success(), "desk-key: {made:?}");
    let out = String::from_utf8_lossy(&made.stdout);
    let desk: Value = out
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str(l).ok())
        .unwrap_or_else(|| panic!("desk-key printed no JSON: {out}"));
    let key_text = std::fs::read_to_string(root.join("compose/approval-desk.key")).unwrap();
    assert!(!out.contains(key_text.trim()), "desk-key printed the key");
    let desk_key_id = desk["desk_key_id"]
        .as_str()
        .expect("desk_key_id")
        .to_string();
    let t0 = Instant::now();
    let child = Command::new(&bin)
        .arg("daemon")
        .env_remove("AIEN_GPU_BACKEND")
        .env_remove("AIEN_REQUIRE_BLACKWELL")
        .env("AIEN_MODEL_PATH", model.join("model.safetensors"))
        .env("AIEN_TOKENIZER_PATH", model.join("tokenizer.json"))
        .env("AIEN_REQUIRE_CHECKPOINT", "1")
        .env("AIEN_COMPOSE_DIR", root.join("compose"))
        .env("AIEN_RUNTIME_STATE_DIR", root.join("state"))
        .env("AIEN_PROVENANCE_DIR", root.join("prov"))
        .env("AIEN_RUNTIME_SOCK", &sock)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(std::fs::File::create(root.join("daemon.err")).unwrap())
        .spawn()
        .expect("spawn aien-cli daemon");
    let mut d = Daemon {
        child,
        root: root.clone(),
        sock: sock.clone(),
        desk_key_id,
        log: log.clone(),
        started_ms: 0,
        _keep: keep,
    };
    // Started = socket present and the declared warm-up line printed (run-campaign.sh rule).
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if sock.exists() && text.contains("Warm-up:") {
            break;
        }
        if let Ok(Some(st)) = d.child.try_wait() {
            panic!(
                "DAEMON_NOT_STARTED {row}: exited {st}; log {}",
                log.display()
            );
        }
        if t0.elapsed() > Duration::from_secs(600) {
            panic!(
                "DAEMON_NOT_STARTED {row}: no warm-up within 600 s; log {}",
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_millis(500));
    d.started_ms = t0.elapsed().as_millis();
    std::fs::create_dir_all(d.ws().join("notes")).unwrap();
    Some(d)
}

/// `records_total`, read twice: the read appends nothing.
fn count(c: &LedgerClient) -> u64 {
    let a = c.records_total().expect("ComposeRecall");
    let b = c.records_total().expect("ComposeRecall");
    assert_eq!(a, b, "ComposeRecall must not append");
    a
}

/// Every `effect` record in the journal, whatever grant it names.
fn all_effects(c: &LedgerClient) -> usize {
    // Effect phase records (intent, ack) on any grant. The daemon also writes
    // effect-class bookkeeping (replay claims, compose-commit records), which
    // carry no `phase` and are not effects.
    let r = c.recall(&[], None).unwrap();
    r["host"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["note"] == "effect")
        .filter_map(|h| serde_json::from_str::<Value>(h["text"].as_str()?).ok())
        .filter(|t| t.get("phase").is_some())
        .count()
}

/// Host records of kind `effect` that name authorization `grant`.
fn effects_for(c: &LedgerClient, grant: u64) -> Vec<Value> {
    let r = c.recall(&[], None).unwrap();
    r["host"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["note"] == "effect")
        .filter_map(|h| serde_json::from_str::<Value>(h["text"].as_str()?).ok().map(|t| (h, t)))
        .filter(|(_, t)| t["authorization"].as_u64() == Some(grant))
        .map(|(h, t)| json!({"id": h["id"], "digest": h["digest"], "verified": h["verified"], "text": t}))
        .collect()
}

/// id, digest, verified and text of the named records.
fn records(c: &LedgerClient, ids: &[u64]) -> Vec<Value> {
    let r = c.recall(ids, None).unwrap();
    assert!(r["missing"].as_array().unwrap().is_empty(), "{r}");
    r["cited"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| json!({"id": v["id"], "digest": v["digest"], "verified": v["verified"], "note": v["note"],
            "text": v["text"].as_str().and_then(|t| serde_json::from_str::<Value>(t).ok())}))
        .collect()
}

fn save(d: &Daemon, row: &str, mut v: Value) {
    v["row"] = json!(row);
    v["boundary"] = json!(LEDGER_BOUNDARY);
    v["daemon"] = json!({"log": d.log, "home": d.root.join("compose"), "start_ms": d.started_ms,
        "aien_bin": std::env::var("AIEN_BIN").ok()});
    std::fs::write(
        d.root.join("receipt.json"),
        serde_json::to_vec_pretty(&v).unwrap(),
    )
    .unwrap();
    println!("{row}: {v}");
}

fn input(id: &str, kind: &str, trust: &str, body: &str) -> InputRecord {
    serde_json::from_value(json!({
        "input_id": id, "content_kind": kind, "trust": trust,
        "source": {"kind": "operator", "id": "drake"}, "origin": "operator:turn-0",
        "content_digest": format!("sha256:{}", hex(body.as_bytes())), "trace_id": TRACE,
        "parent_id": null, "derived_from": []
    }))
    .unwrap()
}

fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}})
}

fn turn(calls: Vec<Value>) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": calls})
}

/// Adapter + ledger authority over the daemon's workspace.
/// The adapter's clock is pinned to the synthetic epoch these rows use (`now` 10, `expires_at`
/// 1000), as in `compose_ledger.rs` (sovereign-core #260 checks expiry before the handoff).
fn ledger(d: &Daemon) -> ComposeLedgerAuthority {
    let mut a = AienAuthority::new(d.ws()).unwrap();
    a.set_clock(std::sync::Arc::new(|| 10));
    let shared = AienShared::new(a);
    ComposeLedgerAuthority::new(shared, d.ws(), &d.sock, d.desk()).unwrap()
}

/// `(replay_claim, grant_id)` of a receipt that reached the grant.
fn claim_grant(rc: &Value) -> (u64, u64) {
    (
        rc["handoff"]["replay_claim"]
            .as_u64()
            .expect("replay_claim"),
        rc["grant_id"].as_u64().expect("grant_id"),
    )
}

/// The approved `{content, path}` digest (INTERPLANE form) and the compose form.
fn approved_sha(path: &str, content: &str) -> String {
    hex(json!({"content": content, "path": path})
        .to_string()
        .as_bytes())
}
fn compose_sha(path: &str, content: &str) -> String {
    hex(format!("filename: {path}\n{content}").as_bytes())
}

/// A handoff for `ComposeApprovedProposal`, MAC'd with `key` (`None`: no MAC).
fn handoff(key: Option<&DeskKey>, b: &ApprovalBinding, content: &str) -> Value {
    json!({"request_id": b.request_id, "trace_id": b.trace_id, "approval_id": b.approval_id,
        "approver": b.approver, "path": b.path, "content": content,
        "approved_proposal_sha256": b.approved_proposal_sha256, "content_sha256": b.content_sha256,
        "approval_mac": key.map(|k| k.mac(b)).unwrap_or_default(), "requirements": BOUND_REQUIREMENTS,
        "requirements_mac": key.map(|k| k.requirements_mac(b, BOUND_REQUIREMENTS)).unwrap_or_default()})
}

fn binding(
    d: &Daemon,
    rid: &str,
    aid: &str,
    approver: &str,
    path: &str,
    content: &str,
) -> ApprovalBinding {
    ApprovalBinding {
        trace_id: TRACE.into(),
        request_id: rid.into(),
        approval_id: aid.into(),
        approver: approver.into(),
        path: path.into(),
        content_sha256: hex(content.as_bytes()),
        approved_proposal_sha256: approved_sha(path, content),
        desk_key_id: d.desk_key_id.clone(),
        workspace: d.ws().to_str().unwrap().into(),
    }
}

fn pipe<'a>(rt: &'a mut ComposeLedgerAuthority, l: &ComposeLedgerAuthority) -> Pipeline<'a> {
    let table = l.shared().with(|a| a.mapping_table(true));
    let mut p = Pipeline::new(
        DialectRegistry::with_defaults(),
        table,
        rt,
        Limits::default(),
        RequestLedger::new(),
    );
    p.register_input(input(
        "in-user",
        "user_request",
        "user_supplied",
        "write the summary",
    ))
    .unwrap();
    p
}

/// Propose `write_file path content` as request `id`; it must stop at requires_approval.
fn propose(p: &mut Pipeline<'_>, n: u64, id: &str, path: &str, content: &str) {
    let out = p.run_turn(
        "openai",
        "scripted",
        &turn(vec![call(
            id,
            "write_file",
            json!({"path": path, "content": content}),
        )]),
        TRACE,
        n,
    );
    assert_eq!(
        out.results[0].status,
        ResultStatus::RequiresApproval,
        "{:?}",
        out.results[0]
    );
}

/// Host approves request `id` (one AIEN grant) as `drake` and continues it.
fn approve(
    p: &mut Pipeline<'_>,
    l: &ComposeLedgerAuthority,
    id: &str,
) -> (ToolResult, aien_mcp::ApprovalGrant) {
    let pa = p.pending_approval(TRACE, id).expect("pending");
    let g = l
        .shared()
        .with(|a| a.issue_approval(&pa.capability_request, 1_000))
        .unwrap();
    let (r, _) = l
        .continue_approval(p, TRACE, id, &g, 10, NOW, Some("drake"))
        .expect("continued");
    (r, g)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// The old route: a caller notes its own `authorization` grant (workspace, path, confined
/// target, compose-form digest) for exactly the write the host will approve. Since
/// sovereign-core #261 (#296) the daemon refuses every caller-written `authorization` note;
/// returns the refusal text.
fn client_minted(c: &LedgerClient, ws: &str, path: &str, content: &str) -> String {
    let target = Path::new(ws).join(path).to_str().unwrap().to_string();
    let text = json!({"proposal_sha256": compose_sha(path, content), "path": path,
        "content_sha256": hex(content.as_bytes()), "approver": "attacker", "workspace": ws,
        "target": target, "prior_sha256": null});
    match c.note("authorization", &text.to_string(), &[]) {
        Ok(n) => panic!("ATTACK SUCCEEDED: the daemon accepted a client-minted grant note: {n}"),
        Err(e) => e,
    }
}

/// Host approves `id`; the outcome as (Ok?, error text). A pipeline refusal counts as not Ok.
fn approve_outcome(p: &mut Pipeline<'_>, l: &ComposeLedgerAuthority, id: &str) -> (bool, String) {
    let pa = p.pending_approval(TRACE, id).expect("pending");
    let g = l
        .shared()
        .with(|a| a.issue_approval(&pa.capability_request, 1_000))
        .unwrap();
    match l.continue_approval(p, TRACE, id, &g, 10, NOW, Some("drake")) {
        Ok((r, _)) => (
            r.status == ResultStatus::Ok,
            r.error.map(|e| e.message).unwrap_or_default(),
        ),
        Err(e) => (false, format!("pipeline refused: {e:?}")),
    }
}

/// Item (1): a client-minted grant note for exactly the approved write is refused by the daemon
/// (sovereign-core #261), and the daemon refuses the handoff (the request id was already
/// claimed). Nothing is written, no intent opens on the client grant, the adapter does not
/// fall back to it.
#[test]
#[ignore = "NOT_RUN unless AIEN_BIN names an aien-cli binary: run with --ignored"]
fn x1_client_minted_grant_unused_when_the_handoff_refuses() {
    let _g = lock();
    let Some(d) = daemon("x1") else { return };
    let c = d.client();
    let ws = d.ws().to_str().unwrap().to_string();
    let path = "minted.txt";
    let before_note = count(&c);
    let minted = client_minted(&c, &ws, path, EXPECTED);
    assert!(
        minted.contains("sovereign-core #261"),
        "client-minted note refused for another reason: {minted}"
    );
    assert_eq!(
        count(&c),
        before_note,
        "a refused client-minted note appended records"
    );
    // Request id m1 is claimed first by a real, desk-signed handoff for other content.
    let k = DeskKey::load(&d.desk()).unwrap();
    let b = binding(
        &d,
        "m1",
        "aien-approval:other",
        "drake",
        "other.txt",
        "other\n",
    );
    c.approved(&handoff(Some(&k), &b, "other\n"), &ws)
        .expect("first handoff commits");
    drop(k);
    let before = count(&c);
    let l = ledger(&d);
    let mut rt = l.clone();
    let mut p = pipe(&mut rt, &l);
    propose(&mut p, 0, "m1", path, EXPECTED);
    let (ok, err) = approve_outcome(&mut p, &l, "m1");
    assert!(
        !ok,
        "ATTACK SUCCEEDED: write reported Ok after a refused handoff"
    );
    assert!(
        !d.ws().join(path).exists(),
        "ATTACK SUCCEEDED: file written"
    );
    assert_eq!(
        all_effects(&c),
        0,
        "ATTACK SUCCEEDED: an effect record exists"
    );
    assert_eq!(l.grants_minted(), 0);
    assert_eq!(count(&c), before, "a refused handoff appended records");
    let rc = l.receipts().last().cloned().unwrap();
    assert_eq!(rc["state"], "REFUSED");
    println!("x1: PASS refused: {err}");
}

/// No fallback: the daemon is gone when the host approves. Nothing is written anywhere in the
/// workspace (the in-process provider never runs the write).
#[test]
#[ignore = "NOT_RUN unless AIEN_BIN names an aien-cli binary: run with --ignored"]
fn x2_no_fallback_write_when_the_daemon_is_gone() {
    let _g = lock();
    let Some(mut d) = daemon("x2") else { return };
    let l = ledger(&d);
    let mut rt = l.clone();
    let mut p = pipe(&mut rt, &l);
    propose(&mut p, 0, "g1", "gone.txt", EXPECTED);
    d.child.kill().unwrap();
    d.child.wait().unwrap();
    let (ok, err) = approve_outcome(&mut p, &l, "g1");
    assert!(!ok, "ATTACK SUCCEEDED: write reported Ok with no daemon");
    assert!(
        !d.ws().join("gone.txt").exists(),
        "ATTACK SUCCEEDED: fallback write"
    );
    let left: Vec<_> = std::fs::read_dir(d.ws())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "notes")
        .collect();
    assert!(left.is_empty(), "workspace changed: {left:?}");
    println!("x2: PASS refused: {err}");
}

/// The effect names the daemon's own grant and the compose-form digest; a client-minted grant
/// note for the same write is refused first (sovereign-core #261), so no other grant exists.
#[test]
#[ignore = "NOT_RUN unless AIEN_BIN names an aien-cli binary: run with --ignored"]
fn x3_effect_uses_the_daemon_grant_not_a_client_one() {
    let _g = lock();
    let Some(d) = daemon("x3") else { return };
    let c = d.client();
    let ws = d.ws().to_str().unwrap().to_string();
    let minted = client_minted(&c, &ws, "summary.txt", EXPECTED);
    assert!(minted.contains("sovereign-core #261"), "{minted}");
    let l = ledger(&d);
    let mut rt = l.clone();
    let mut p = pipe(&mut rt, &l);
    propose(&mut p, 0, "u1", "summary.txt", EXPECTED);
    let (ok, err) = approve_outcome(&mut p, &l, "u1");
    assert!(ok, "honest write failed: {err}");
    let rc = l.receipts().last().cloned().unwrap();
    let grant = rc["grant_id"].as_u64().unwrap();
    assert_eq!(rc["handoff"]["approved_grant"], json!(grant));
    let g = &records(&c, &[grant])[0];
    assert_eq!(g["text"]["approved_grant"], 1, "the daemon wrote the grant");
    let csha = compose_sha("summary.txt", EXPECTED);
    assert_eq!(rc["proposal_sha256"], json!(csha));
    assert_eq!(rc["handoff"]["state"], "COMMITTED");
    assert_eq!(
        effects_for(&c, grant).len(),
        2,
        "intent + ack on the daemon grant"
    );
    assert_eq!(
        all_effects(&c),
        2,
        "an effect names a grant other than the daemon's"
    );
    assert_eq!(rc["state"], "DONE");
    println!("x3: PASS grant #{grant} (client-minted note refused: {minted})");
}

/// The write's temporary file has a predictable name in the target directory. Planted there as a
/// symlink to a file outside the workspace, the write must not land outside.
#[test]
#[ignore = "NOT_RUN unless AIEN_BIN names an aien-cli binary: run with --ignored"]
fn x4_planted_temp_symlink_does_not_redirect_the_write() {
    let _g = lock();
    let Some(d) = daemon("x4") else { return };
    let outside = d.root.join("outside.txt");
    std::fs::write(&outside, "ORIGINAL\n").unwrap();
    let tmp = d.ws().join(format!(
        ".summary.txt.interplane-{}.tmp",
        std::process::id()
    ));
    std::os::unix::fs::symlink(&outside, &tmp).unwrap();
    let l = ledger(&d);
    let mut rt = l.clone();
    let mut p = pipe(&mut rt, &l);
    propose(&mut p, 0, "s1", "summary.txt", EXPECTED);
    let (ok, err) = approve_outcome(&mut p, &l, "s1");
    let rc = l.receipts().last().cloned().unwrap();
    let now = std::fs::read_to_string(&outside).unwrap();
    println!(
        "x4: ok={ok} state={} outside={now:?} err={err}",
        rc["state"]
    );
    assert_eq!(
        now, "ORIGINAL\n",
        "ATTACK SUCCEEDED: the approved bytes landed outside the workspace"
    );
}
