//! Export of one live adapter run as a provenance bundle (`provenance/` verifier input).
//!
//! After a live row against a real `aien-cli daemon`, [`export_bundle`] writes a directory that
//! `cd provenance && cargo run -q --locked -- verify <dir>` can judge: the daemon's own
//! `ComposeRecall` views of the claim, the settlement, the grant, the intent and the ack (raw,
//! unchanged), a `daemon-run.json` the harness writes from `/proc`, the daemon's load log, the
//! INTERPLANE trace of this run, the model files the daemon loaded (hard link, or copy), and a
//! `COMPANION.json` naming each by path, size and SHA-256.
//!
//! Fix-the-test slice: with [`LiveRun::slice`] the bundle also carries the harness's task, source pin
//! and test-run records (`vac-test-run/1`, see `crate::fix_the_test`).
//!
//! Honest labels, nothing invented: the proposal here is a SCRIPTED turn (the test wrote the tool
//! call), so the manifest says `proposal_origin: scripted_turn` and the verdict can never be
//! `PASS complete`; it is `PASS_LABELLED_INCOMPLETE missing=link:model_turn`. The model's training
//! history is unknown (`lineage.kind = unknown_pretraining`, with the origin the caller names).
//! No record is signed; the verifier cannot tell an export from a hand-written file (see
//! `provenance/BINDING.md`, Trust). This module reads the daemon and writes files; it grants and
//! decides nothing.
use crate::compose_ledger::LedgerClient;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// What the exporter needs to know about the finished run.
pub struct LiveRun<'a> {
    /// Bundle directory; created, and refused when it already holds a `COMPANION.json`.
    pub out: &'a Path,
    pub client: &'a LedgerClient,
    /// The `aien-cli` binary the daemon was started from.
    pub aien_bin: &'a Path,
    /// Process id of that daemon (the harness spawned it).
    pub daemon_pid: u32,
    /// The daemon's stdout log (contains the model load line and the socket line).
    pub daemon_log: &'a Path,
    pub socket: &'a Path,
    pub workspace: &'a Path,
    /// Directory holding `model.safetensors`, `tokenizer.json` and `config.json`.
    pub model_dir: &'a Path,
    /// Where the weights came from (`model_id`, `revision`), restated and not judged.
    pub model_origin: (&'a str, &'a str),
    /// The label on the scripted "model" in the trace.
    pub model_label: &'a str,
    pub trace_id: &'a str,
    /// `serde_json::to_value` of the pipeline's intent, of its `requires_approval` result and of
    /// the final result of the approved continuation.
    pub intent: &'a Value,
    pub pending: &'a Value,
    pub final_result: &'a Value,
    /// RFC 3339 times of the request and of completion (see [`rfc3339_utc`]).
    pub t_request: &'a str,
    pub t_done: &'a str,
    /// The fix-the-test slice: task, source pin and test-run records (harness evidence).
    pub slice: Option<&'a crate::fix_the_test::TestRunEvidence>,
    /// The M5 slice: the independent judge's signed receipt and the policy it ran under, as the
    /// exact bytes the judge wrote and the operator pinned (never re-serialized).
    pub evaluation: Option<&'a Evaluation>,
    /// A real model turn: the daemon generated the proposal on the model it loaded. When set the
    /// bundle says `proposal_origin: model_generation/2` and retains the request, the response and
    /// the daemon's own generation record; the `model_label` is the id the trace names as sender.
    pub model_turn: Option<&'a ModelTurnRun>,
}

/// One real model turn as the harness saw it (see `LedgerClient::stream_turn`). All bytes are kept
/// exactly: `request` is the JSON `messages` array sent to the daemon, `response` the text it
/// returned.
#[derive(Debug, Clone)]
pub struct ModelTurnRun {
    pub request: Vec<u8>,
    pub response: String,
    pub generation_record: u64,
    pub total_tokens: u64,
    pub max_tokens: u64,
    pub temperature: f64,
    pub started_at: String,
    pub finished_at: String,
}

/// The judge's evidence for one change (`rsi-eval/2`, spark-rsi docs/RECEIPT-V2.md).
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub receipt: Vec<u8>,
    pub policy: Vec<u8>,
}

/// What was written.
#[derive(Debug)]
pub struct ExportReport {
    pub dir: PathBuf,
    pub request_id: String,
    pub claim: u64,
    pub ack: u64,
    pub executable_sha256: String,
    pub model_hardlinked: bool,
}

pub fn sha256_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

fn sha256_file(p: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time in seconds (proleptic Gregorian, UTC).
pub fn rfc3339_utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
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

/// Now, as RFC 3339 UTC.
pub fn now_rfc3339() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_utc(s)
}

/// Field 22 of `/proc/<pid>/stat` (start time in clock ticks since boot), the same identity the
/// daemon writes into its replay claim as `executor.start`.
pub fn proc_start_ticks(pid: u32) -> Result<u64, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| e.to_string())?;
    // `comm` may hold spaces and parentheses: split after the LAST ')'.
    let rest = stat.rsplit_once(')').ok_or("stat: no ')'")?.1;
    rest.split_whitespace()
        .nth(19)
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| "stat: no start time".into())
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().ok_or("no parent")?).map_err(|e| e.to_string())?;
    std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))
}

/// Write a record file and list it in the manifest by path, digest and size.
fn put(
    dir: &Path,
    records: &mut Map<String, Value>,
    name: &str,
    rel: &str,
    bytes: Vec<u8>,
) -> Result<(), String> {
    write(dir, rel, &bytes)?;
    records.insert(
        name.into(),
        json!({"path": rel, "sha256": sha256_hex(&bytes), "bytes": bytes.len()}),
    );
    Ok(())
}

fn pretty(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap_or_default()
}

/// The raw `ComposeRecall` view (`cited[]`) of one record id.
fn view(cited: &[Value], id: u64) -> Result<Value, String> {
    cited
        .iter()
        .find(|v| v["id"].as_u64() == Some(id))
        .cloned()
        .ok_or_else(|| format!("record {id} not returned by ComposeRecall"))
}

fn text_of(v: &Value) -> Value {
    v["text"]
        .as_str()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null)
}

/// Write the bundle and return what it holds. Errors say what was missing; nothing is guessed.
pub fn export_bundle(run: &LiveRun<'_>) -> Result<ExportReport, String> {
    if run.out.join("COMPANION.json").exists() {
        return Err(format!(
            "{} already holds a COMPANION.json (a sealed record is never rewritten)",
            run.out.display()
        ));
    }
    std::fs::create_dir_all(run.out).map_err(|e| e.to_string())?;
    let rc = &run.final_result["data"]["receipt"];
    let id_of = |k: &str| {
        rc[k]
            .as_u64()
            .or_else(|| rc["handoff"][k].as_u64())
            .ok_or_else(|| format!("receipt has no {k}"))
    };
    let (claim, grant, intent, ack) = (
        id_of("replay_claim")?,
        id_of("grant_id")?,
        id_of("intent_id")?,
        id_of("ack_record_id")?,
    );
    let request_id = run.final_result["request_id"]
        .as_str()
        .ok_or("final result has no request_id")?
        .to_string();

    // The daemon's own export command, over the window claim..=ack.
    let ids: Vec<u64> = (claim..=ack).collect();
    let recalled = run.client.recall(&ids, None)?;
    if !recalled["missing"].as_array().is_some_and(|m| m.is_empty()) {
        return Err(format!(
            "ComposeRecall missing records: {}",
            recalled["missing"]
        ));
    }
    let cited = recalled["cited"]
        .as_array()
        .ok_or("ComposeRecall: no cited")?;
    let committed = cited
        .iter()
        .find(|v| {
            let t = text_of(v);
            t["approved_submission"] == "committed" && t["claim"].as_u64() == Some(claim)
        })
        .cloned()
        .ok_or("no committed settlement record for the claim")?;

    let mut records = Map::new();
    put(
        run.out,
        &mut records,
        "ledger_claim",
        "records/aien/ledger-claim.json",
        pretty(&view(cited, claim)?),
    )?;
    put(
        run.out,
        &mut records,
        "ledger_committed",
        "records/aien/ledger-committed.json",
        pretty(&committed),
    )?;
    put(
        run.out,
        &mut records,
        "ledger_grant",
        "records/aien/ledger-grant.json",
        pretty(&view(cited, grant)?),
    )?;
    put(
        run.out,
        &mut records,
        "ledger_intent",
        "records/aien/ledger-intent.json",
        pretty(&view(cited, intent)?),
    )?;
    put(
        run.out,
        &mut records,
        "ledger_ack",
        "records/aien/ledger-ack.json",
        pretty(&view(cited, ack)?),
    )?;

    // The daemon states whether its compose library is native (sovereign-core #346). An older
    // daemon returns neither field: then `aien.native` is absent and the verdict says so.
    let native = recalled["compose_native"].as_bool().map(|claimed| {
        let sha = recalled["omega_sha"].as_str().unwrap_or("");
        json!({"claimed": claimed, "omega_sha": sha})
    });
    if native.is_some() {
        put(
            run.out,
            &mut records,
            "compose_recall",
            "records/aien/compose-recall.json",
            pretty(&recalled),
        )?;
    }
    if let Some(s) = run.slice {
        put(
            run.out,
            &mut records,
            "task",
            "records/task/task.json",
            pretty(&s.task),
        )?;
        put(
            run.out,
            &mut records,
            "source_pin",
            "records/task/source-pin.json",
            pretty(&s.source_pin),
        )?;
        put(
            run.out,
            &mut records,
            "test_run_record",
            "records/task/test-run.json",
            pretty(&s.test_run),
        )?;
        put(
            run.out,
            &mut records,
            "test_stdout",
            "records/task/test-stdout.txt",
            s.stdout.clone(),
        )?;
        put(
            run.out,
            &mut records,
            "test_stderr",
            "records/task/test-stderr.txt",
            s.stderr.clone(),
        )?;
    }
    if let Some(ev) = run.evaluation {
        put(
            run.out,
            &mut records,
            "evaluation_receipt",
            "records/evaluation/receipt.json",
            ev.receipt.clone(),
        )?;
        put(
            run.out,
            &mut records,
            "evaluation_policy",
            "records/evaluation/policy.json",
            ev.policy.clone(),
        )?;
    }
    let exe_sha = sha256_file(run.aien_bin)?;
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let daemon_run = json!({"kind": "aien-daemon-run", "pid": run.daemon_pid,
        "start_ticks": proc_start_ticks(run.daemon_pid)?,
        "socket": run.socket.to_str().ok_or("socket path")?,
        "workspace": run.workspace.to_str().ok_or("workspace path")?,
        "executable_sha256": exe_sha, "boot_id": boot_id,
        "note": "written by the run harness from /proc/<pid>/stat and the spawned child pid, not by the daemon"});
    put(
        run.out,
        &mut records,
        "daemon_run",
        "records/aien/daemon-run.json",
        pretty(&daemon_run),
    )?;
    put(
        run.out,
        &mut records,
        "aien_load_log",
        "records/aien/daemon-load.log",
        std::fs::read(run.daemon_log).map_err(|e| format!("daemon log: {e}"))?,
    )?;

    // The trace: the request the pipeline admitted, its requires_approval result, the final result.
    let env =
        |n: u32, parent: Option<&str>, src: (&str, &str), dst: (&str, &str), t: &str, p: &Value| {
            let mut e = json!({"interplane_version": "0.1", "message_id": format!("m-{n}"),
            "trace_id": run.trace_id, "timestamp": t,
            "source": {"kind": src.0, "id": src.1}, "destination": {"kind": dst.0, "id": dst.1},
            "payload": p});
            if let Some(pp) = parent {
                e["parent_id"] = json!(pp);
            }
            e
        };
    let model = ("model", run.model_label);
    let trace = json!([
        env(
            1,
            None,
            model,
            ("runtime", "aien"),
            run.t_request,
            run.intent
        ),
        env(
            2,
            Some("m-1"),
            ("runtime", "aien"),
            model,
            run.t_request,
            run.pending
        ),
        env(
            3,
            Some("m-1"),
            ("runtime", "aien"),
            model,
            run.t_done,
            run.final_result
        ),
    ]);
    for e in trace.as_array().into_iter().flatten() {
        interplane_core::validate_envelope_value(e)
            .map_err(|c| format!("trace envelope invalid {c:?}"))?;
    }
    put(
        run.out,
        &mut records,
        "interplane_trace",
        "records/interplane/trace.json",
        pretty(&trace),
    )?;

    // The model files the daemon loaded: hard link when on one filesystem, else a copy.
    let mut hardlinked = true;
    for (name, file) in [
        ("export_weights", "model.safetensors"),
        ("export_tokenizer", "tokenizer.json"),
        ("export_config", "config.json"),
    ] {
        let src = std::fs::canonicalize(run.model_dir.join(file))
            .map_err(|e| format!("{}: {e}", run.model_dir.join(file).display()))?;
        let rel = format!("records/export/{file}");
        let dst = run.out.join(&rel);
        std::fs::create_dir_all(dst.parent().ok_or("no parent")?).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&dst);
        if std::fs::hard_link(&src, &dst).is_err() {
            hardlinked = false;
            std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        }
        records.insert(
            name.into(),
            json!({"path": rel, "sha256": sha256_file(&dst)?,
                "bytes": std::fs::metadata(&dst).map_err(|e| e.to_string())?.len()}),
        );
    }

    // A real model turn: the request and response bytes, the daemon's own record of the
    // generation (exported with its own read command) and a model-turn/2 record that names the
    // digests that tie them to the weights and to the trace.
    if let Some(t) = run.model_turn {
        let log_text = std::fs::read_to_string(run.daemon_log).unwrap_or_default();
        let backend_line = log_text
            .lines()
            .find(|l| l.contains("Backend:"))
            .map(|l| l.trim().to_string())
            .ok_or("the daemon log names no Backend: line; the turn's backend would be a guess")?;
        let gview = run.client.recall(&[t.generation_record], None)?;
        if !gview["missing"].as_array().is_some_and(|m| m.is_empty()) {
            return Err(format!(
                "ComposeRecall has no generation record {}: {}",
                t.generation_record, gview["missing"]
            ));
        }
        let gcited = gview["cited"].as_array().ok_or("ComposeRecall: no cited")?;
        put(
            run.out,
            &mut records,
            "ledger_generation",
            "records/aien/ledger-generation.json",
            pretty(&view(gcited, t.generation_record)?),
        )?;
        put(
            run.out,
            &mut records,
            "model_request",
            "records/model/request.json",
            t.request.clone(),
        )?;
        let weights = records["export_weights"]["sha256"]
            .as_str()
            .ok_or("export_weights not recorded")?
            .to_string();
        let turn = json!({"record": "model-turn/2", "dialect": "aien_legacy",
            "model": run.model_label, "model_id": run.model_origin.0,
            "trace_id": run.trace_id, "turn": 0,
            "input": t.response,
            "generation_record_id": t.generation_record,
            "generation_record_source": "TurnFinished.generation_record",
            "request_record": "model_request",
            "request_sha256": sha256_hex(&t.request),
            "response_sha256": sha256_hex(t.response.as_bytes()),
            "weights_sha256": weights,
            "backend": backend_line,
            "sampling": {"max_tokens": t.max_tokens, "temperature": t.temperature,
                "total_tokens": t.total_tokens},
            "stream_started": t.started_at, "stream_finished": t.finished_at});
        put(
            run.out,
            &mut records,
            "model_turn",
            "records/interplane/model-turn.json",
            pretty(&turn),
        )?;
    }

    let mut missing: Vec<&str> = if run.model_turn.is_some() {
        vec![]
    } else {
        vec!["link:model_turn"]
    };
    if run.slice.is_some() && native.as_ref().is_none_or(|n| n["claimed"] != true) {
        missing.push("link:native");
    }
    let (origin, fixture_note, reason) = if run.model_turn.is_some() {
        ("model_generation/2",
         "Real daemon; the ledger records are the daemon's own ComposeRecall output (pretty-printed only); daemon_run.json and the 3-message trace are written by the test harness, including their timestamps. The model turn is REAL: the daemon generated the tool-call text (StreamTurn) and wrote its own generation record; the request bytes and timestamps are the harness's.",
         "the proposal text was generated by the model this daemon loaded; the daemon's own generation record, the request and response bytes, the weights digest and the parsed request all chain to the effect")
    } else {
        ("scripted_turn",
         "Real daemon; the five ledger records are the daemon's own ComposeRecall output (pretty-printed only); daemon_run.json and the 3-message trace are written by the test harness, including their timestamps. The model turn is SCRIPTED by the test (the tool call is a test constant), so authorship of the proposal is not claimed; the model only loaded.",
         "the INTERPLANE model turn is scripted by the test: nothing links this model to the effect's content")
    };
    let mut companion = json!({
        "kind": "interplane-provenance-companion", "schema": 0,
        "fixture": {"class": "real", "note": fixture_note},
        "lineage": {"kind": "unknown_pretraining",
            "origin": {"model_id": run.model_origin.0, "model_id_source": "caller-asserted (AIEN_LEDGER_MODEL_ID or default; not checked against the model directory)", "revision": run.model_origin.1,
                "note": "weights as downloaded; no training evidence is held"}},
        "export": {"format": "huggingface", "chat_template": Value::Null},
        "load_support": {"declared": "supported_structural"},
        "aien": {"candidate_id": Value::Null, "executable": "aien-cli", "executable_sha256": exe_sha,
            "note": "no frozen AIEN candidate: no candidate_manifest record; CPU-reference backend as printed in the daemon log; whether the compose library was native-linked is reported only when the daemon returns compose_native (sovereign-core #346); aien.native then names it"},
        "interplane": {"trace_id": run.trace_id, "request_id": request_id},
        "effect": {"binding": "aien-ledger-slice/1", "approval_binding": "aien.approval.v2",
            "proposal_origin": origin},
        "completeness": {"state": if missing.is_empty() { "complete" } else { "incomplete" }, "missing": missing,
            "reason": reason},
        "records": Value::Object(records),
    });
    if let Some(n) = native {
        companion["aien"]["native"] = n;
    }
    if let Some(s) = run.slice {
        companion["test_run"] = json!({"binding": "vac-test-run/1", "task_id": s.task_id,
            "record": "test_run_record"});
    }
    if run.evaluation.is_some() {
        companion["evaluation"] = json!({"binding": "rsi-eval/2",
            "receipt": "evaluation_receipt", "policy": "evaluation_policy",
            "note": if run.model_turn.is_some() { "the change was proposed by the model turn and evaluated by the separate judge process; the judge's public key is supplied to the verifier out of band" } else { "the change was proposed by spark-rsi and evaluated by the separate judge process; the judge's public key is supplied to the verifier out of band" }});
    }
    write(run.out, "COMPANION.json", &pretty(&companion))?;
    Ok(ExportReport {
        dir: run.out.to_path_buf(),
        request_id,
        claim,
        ack,
        executable_sha256: exe_sha,
        model_hardlinked: hardlinked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_791_417_600), "2026-10-08T00:00:00Z");
    }

    #[test]
    fn own_start_ticks_are_readable() {
        assert!(proc_start_ticks(std::process::id()).unwrap() > 0);
    }

    #[test]
    fn refuses_to_overwrite_a_sealed_bundle() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("COMPANION.json"), b"{}").unwrap();
        let c = LedgerClient::new(d.path().join("none.sock"));
        let v = json!(null);
        let r = export_bundle(&LiveRun {
            out: d.path(),
            client: &c,
            aien_bin: Path::new("/nonexistent"),
            daemon_pid: 1,
            daemon_log: Path::new("/nonexistent"),
            socket: Path::new("/x"),
            workspace: Path::new("/x"),
            model_dir: Path::new("/x"),
            model_origin: ("m", "r"),
            model_label: "m",
            trace_id: "t",
            intent: &v,
            pending: &v,
            final_result: &v,
            t_request: "t",
            t_done: "t",
            slice: None,
            evaluation: None,
            model_turn: None,
        });
        assert!(r.unwrap_err().contains("never rewritten"));
    }
}
