//! `write_file` through AIEN's durable effect ledger (NEXT-PHASE-2 path of `aien-cli daemon`).
//!
//! [`ComposeLedgerAuthority`] wraps [`AienShared`]: the approval desk, `decide` and every read are
//! the tested in-process adapter, unchanged. Only an authorized `write_file` takes a different
//! route: instead of the in-process `EffectLane` provider, the adapter asks the running daemon on
//! its control socket for a durable grant (`ComposeNote` kind `authorization`), a durable intent
//! (`ComposeEffectIntent`, where the daemon checks stop, revoke, spent and stale), writes the bytes
//! itself (tmp + rename inside the workspace), and reports back (`ComposeEffectAck`, where the
//! daemon reads the disk and records the state). Grants, intents and acks live in the daemon's
//! Cortex journal (`$AIEN_COMPOSE_DIR`).
//!
//! Limits: the daemon's compose.verify / AEGIS / J-Space / World commit run only inside
//! `RunComposeTask` (the daemon's own model proposer); they are NOT reached here. Every result says
//! so ([`LEDGER_BOUNDARY`]). The socket client speaks the daemon's JSON line protocol directly
//! (`aien-runtime` is not a dependency: its build scripts need the omega libraries).
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aien_mcp::ApprovalGrant;
use interplane_core::*;
use interplane_crossveil::{CallContext, ObservedRecord, Pipeline, Refusal, RuntimeAuthority};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{AienShared, RUNTIME_ID};

/// The label every receipt of this authority carries.
pub const LEDGER_BOUNDARY: &str = "boundary = durable effect ledger + Cortex journal (NEXT-PHASE-2 path); verify step NOT exercised";
/// Approver recorded in the grant when the host names none.
pub const DEFAULT_APPROVER: &str = "interplane-host";

/// sha256 hex (the daemon's digest form: lowercase, no prefix).
pub fn sha256_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// sha256 of a file, `None` when absent; an error for anything that is not a regular file
/// (the daemon's `effects::file_sha256` rule).
pub fn file_sha256(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("stat {}: {e}", path.display())),
        Ok(m) if !m.file_type().is_file() => {
            Err(format!("{} is not a regular file", path.display()))
        }
        Ok(_) => fs::read(path)
            .map(|b| Some(sha256_hex(&b)))
            .map_err(|e| format!("read {}: {e}", path.display())),
    }
}

/// This process as the effect executor: pid and start ticks (/proc/self/stat field 22).
pub fn self_executor() -> (u32, u64) {
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let start = stat
        .rfind(')')
        .and_then(|i| stat[i + 1..].split_whitespace().nth(19))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (std::process::id(), start)
}

static OPERATION: AtomicU64 = AtomicU64::new(0);

/// Newline-JSON client for the daemon's control socket (`aien-runtime` client.rs framing).
#[derive(Clone, Debug)]
pub struct LedgerClient {
    socket: PathBuf,
}

impl LedgerClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// One command in, one response out. `Ok` is the response's single variant body keyed by
    /// name (e.g. `{"ComposeNoted": {...}}`); a daemon `Error` is `Err` with its text.
    pub fn send(&self, command: Value) -> Result<Value, String> {
        // Same-user check: the socket file must be owned by this process's user.
        let me = fs::metadata("/proc/self").map_err(|e| e.to_string())?.uid();
        let owner = fs::metadata(&self.socket)
            .map_err(|e| format!("socket {}: {e}", self.socket.display()))?
            .uid();
        if owner != me {
            return Err(format!(
                "refusing socket {}: owned by uid {owner}",
                self.socket.display()
            ));
        }
        let mut s = UnixStream::connect(&self.socket)
            .map_err(|e| format!("connect {}: {e}", self.socket.display()))?;
        s.set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(|e| e.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let op = now.as_nanos() as u64 + OPERATION.fetch_add(1, Ordering::SeqCst);
        let env = json!({"protocol_version": 1, "request_id": now.as_millis() as u64,
            "operation_id": op, "operator_session": 1, "command": command});
        s.write_all(format!("{env}\n").as_bytes())
            .map_err(|e| format!("send: {e}"))?;
        let mut line = String::new();
        BufReader::new(&s)
            .read_line(&mut line)
            .map_err(|e| format!("receive: {e}"))?;
        let v: Value =
            serde_json::from_str(line.trim()).map_err(|e| format!("response: {e}: {line}"))?;
        match v.get("Error") {
            Some(e) => Err(e.as_str().map_or_else(|| e.to_string(), str::to_string)),
            None => Ok(v),
        }
    }

    fn body(&self, command: Value, variant: &str) -> Result<Value, String> {
        let v = self.send(command)?;
        v.get(variant)
            .cloned()
            .ok_or_else(|| format!("expected {variant}, got {v}"))
    }

    /// `ComposeNote`; returns the `ComposeNoted` report (`id`, `digest`, ...).
    pub fn note(&self, kind: &str, text: &str, links: &[u64]) -> Result<Value, String> {
        self.body(
            json!({"ComposeNote": {"kind": kind, "text": text, "links": links}}),
            "ComposeNoted",
        )
    }

    /// `ComposeEffectIntent`; returns the `ComposeNoted` report of the intent record.
    #[allow(clippy::too_many_arguments)]
    pub fn intent(
        &self,
        authorization: u64,
        proposal_sha256: &str,
        path: &str,
        target: &str,
        content_sha256: &str,
        executor: (u32, u64),
    ) -> Result<Value, String> {
        self.body(
            json!({"ComposeEffectIntent": {"authorization": authorization,
                "proposal_sha256": proposal_sha256, "path": path, "target": target,
                "content_sha256": content_sha256, "executor_pid": executor.0,
                "executor_start": executor.1}}),
            "ComposeNoted",
        )
    }

    /// `ComposeEffectAck`; returns the `ComposeNoted` report of the ack record.
    pub fn ack(&self, intent: u64, reported: &Value) -> Result<Value, String> {
        self.body(
            json!({"ComposeEffectAck": {"intent": intent, "reported": reported}}),
            "ComposeNoted",
        )
    }

    /// `ComposeRecall`; returns the `ComposeRecalled` report.
    pub fn recall(&self, ids: &[u64], prefix: Option<u64>) -> Result<Value, String> {
        self.body(
            json!({"ComposeRecall": {"ids": ids, "prefix": prefix}}),
            "ComposeRecalled",
        )
    }

    /// `ComposeControl` (`stop`, `resume`, `revoke`); returns the `ComposeControlled` report.
    pub fn control(
        &self,
        action: &str,
        approver: &str,
        authorization: Option<u64>,
    ) -> Result<Value, String> {
        self.body(
            json!({"ComposeControl": {"action": action, "approver": approver, "authorization": authorization}}),
            "ComposeControlled",
        )
    }

    /// Number of records in the compose home's Cortex journal (`ComposeRecall.records_total`).
    pub fn records_total(&self) -> Result<u64, String> {
        self.recall(&[], None)?["records_total"]
            .as_u64()
            .ok_or_else(|| "ComposeRecalled without records_total".into())
    }
}

type Hook = Box<dyn FnMut(&Path)>;

struct LedgerState {
    client: LedgerClient,
    workspace: PathBuf,
    approvers: HashMap<String, String>,
    before_intent: Option<Hook>,
    grants_minted: u32,
    receipts: Vec<Value>,
}

/// The adapter with `write_file` routed through the daemon's durable effect ledger. Clone it to
/// hand one copy to the `Pipeline` and keep one for the host (same pattern as [`AienShared`]).
#[derive(Clone)]
pub struct ComposeLedgerAuthority {
    shared: AienShared,
    state: Rc<RefCell<LedgerState>>,
}

impl ComposeLedgerAuthority {
    /// `shared` must be enrolled over the same `workspace`; `socket` is `$AIEN_RUNTIME_SOCK`.
    pub fn new(
        shared: AienShared,
        workspace: impl AsRef<Path>,
        socket: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let workspace =
            fs::canonicalize(workspace.as_ref()).map_err(|e| format!("workspace: {e}"))?;
        Ok(Self {
            shared,
            state: Rc::new(RefCell::new(LedgerState {
                client: LedgerClient::new(socket),
                workspace,
                approvers: HashMap::new(),
                before_intent: None,
                grants_minted: 0,
                receipts: vec![],
            })),
        })
    }

    /// The wrapped in-process adapter (approval desk, counters).
    pub fn shared(&self) -> &AienShared {
        &self.shared
    }

    /// A client on the same daemon socket.
    pub fn client(&self) -> LedgerClient {
        self.state.borrow().client.clone()
    }

    /// Grants (authorization records) this authority asked the daemon to write.
    pub fn grants_minted(&self) -> u32 {
        self.state.borrow().grants_minted
    }

    /// Every ledger receipt this authority produced, oldest first.
    pub fn receipts(&self) -> Vec<Value> {
        self.state.borrow().receipts.clone()
    }

    /// Fault-injection seam (the analogue of aien-cli's `fault_hold("before_intent")`): runs after
    /// the grant is recorded and before the intent is asked for, with the absolute target.
    pub fn set_before_intent(&self, hook: impl FnMut(&Path) + 'static) {
        self.state.borrow_mut().before_intent = Some(Box::new(hook));
    }

    /// Host-only: [`AienShared::continue_approval`] with the host approver recorded in the grant
    /// (`None` = [`DEFAULT_APPROVER`]).
    #[allow(clippy::too_many_arguments)]
    pub fn continue_approval(
        &self,
        pipeline: &mut Pipeline<'_>,
        trace: &str,
        request_id: &str,
        grant: &ApprovalGrant,
        now_epoch: u64,
        now: &str,
        approver: Option<&str>,
    ) -> Result<(ToolResult, ObservedRecord), Refusal> {
        let who = approver.unwrap_or(DEFAULT_APPROVER).to_string();
        self.state
            .borrow_mut()
            .approvers
            .insert(request_id.to_string(), who);
        let out = self
            .shared
            .continue_approval(pipeline, trace, request_id, grant, now_epoch, now);
        self.state.borrow_mut().approvers.remove(request_id);
        out
    }

    /// Absolute target for a workspace-relative `path`: no `..`, parent inside the workspace.
    fn target(&self, path: &str) -> Result<PathBuf, String> {
        let root = self.state.borrow().workspace.clone();
        let p = Path::new(path);
        let outside = || format!("path is outside the workspace: {path}");
        if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(outside());
        }
        let full = root.join(p);
        let name = full.file_name().ok_or_else(outside)?.to_owned();
        let parent = full
            .parent()
            .and_then(|d| fs::canonicalize(d).ok())
            .filter(|d| d.starts_with(&root))
            .ok_or_else(outside)?;
        Ok(parent.join(name))
    }

    /// grant -> intent -> atomic write -> ack. `Err` before the intent means nothing was written.
    fn write_through_ledger(
        &self,
        req: &CapabilityRequest,
        approver: &str,
        approval_id: &str,
    ) -> Result<Value, String> {
        let arg = |k: &str| {
            req.arguments
                .get(k)
                .and_then(Value::as_str)
                .ok_or(format!("missing required argument: {k}"))
        };
        let (path, content) = (arg("path")?, arg("content")?);
        let target = self.target(path)?;
        let tgt = target.to_str().ok_or("target is not UTF-8")?.to_string();
        // Canonical proposal: keys in sorted order whatever serde_json's map ordering.
        let mut prop = Map::new();
        prop.insert("content".into(), json!(content));
        prop.insert("path".into(), json!(path));
        let proposal_sha256 = sha256_hex(Value::Object(prop).to_string().as_bytes());
        let content_sha256 = sha256_hex(content.as_bytes());
        let prior = file_sha256(&target)?;
        let own = json!({"tool": "write_file", "args": req.arguments, "approver": approver, "approval_id": approval_id});
        let receipt_sha256 = sha256_hex(own.to_string().as_bytes());
        let text = json!({"proposal_sha256": proposal_sha256, "path": path, "content_sha256": content_sha256,
            "approver": approver, "receipt_sha256": receipt_sha256, "target": tgt, "prior_sha256": prior});
        let client = self.client();
        let grant = client.note("authorization", &text.to_string(), &[])?;
        self.state.borrow_mut().grants_minted += 1;
        let grant_id = grant["id"].as_u64().ok_or("grant without id")?;
        let hook = self.state.borrow_mut().before_intent.take();
        if let Some(mut h) = hook {
            h(&target);
            self.state.borrow_mut().before_intent = Some(h);
        }
        let mut rc = json!({"boundary": LEDGER_BOUNDARY, "request_id": req.request_id.as_str(),
            "approval_id": approval_id, "approver": approver, "path": path, "target": tgt,
            "proposal_sha256": proposal_sha256, "content_sha256": content_sha256, "prior_sha256": prior,
            "receipt_sha256": receipt_sha256, "grant_id": grant_id, "grant_digest": grant["digest"]});
        let intent = match client.intent(
            grant_id,
            &proposal_sha256,
            path,
            &tgt,
            &content_sha256,
            self_executor(),
        ) {
            Ok(i) => i,
            Err(e) => {
                rc["state"] = json!("REFUSED");
                rc["daemon_error"] = json!(e);
                self.state.borrow_mut().receipts.push(rc);
                return Err(e);
            }
        };
        let intent_id = intent["id"].as_u64().ok_or("intent without id")?;
        let written = atomic_write(&target, content.as_bytes());
        let disk = file_sha256(&target);
        let reported = json!({"written_sha256": disk.as_ref().ok().cloned().flatten(),
            "bytes": content.len(), "error": written.as_ref().err()});
        rc["intent_id"] = json!(intent_id);
        rc["intent_digest"] = intent["digest"].clone();
        rc["disk_sha256"] = reported["written_sha256"].clone();
        let ack = client.ack(intent_id, &reported);
        let state = match &ack {
            Ok(a) => {
                rc["ack_record_id"] = a["id"].clone();
                rc["ack_digest"] = a["digest"].clone();
                let id = a["id"].as_u64().unwrap_or(0);
                client
                    .recall(&[id], None)
                    .ok()
                    .and_then(|r| r["cited"][0]["text"].as_str().map(str::to_string))
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .and_then(|v| v["state"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "UNRESOLVED".into())
            }
            Err(e) => {
                rc["ack_error"] = json!(e);
                "UNRESOLVED".into()
            }
        };
        rc["state"] = json!(state);
        self.state.borrow_mut().receipts.push(rc.clone());
        if state == "DONE" {
            Ok(rc)
        } else {
            Err(format!("effect not done: {rc}"))
        }
    }
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.interplane-{}.tmp", std::process::id()));
    let r = fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()))
        .and_then(|_| fs::rename(&tmp, target));
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r.map_err(|e| format!("write {}: {e}", target.display()))
}

impl RuntimeAuthority for ComposeLedgerAuthority {
    fn runtime_id(&self) -> &str {
        RUNTIME_ID
    }

    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision {
        self.shared.decide(req, ctx)
    }

    fn execute(
        &mut self,
        req: &CapabilityRequest,
        decision: &Decision,
        ctx: &CallContext,
    ) -> ToolResult {
        if req.capability != "write_file" || !decision.is_authorized() {
            return self.shared.execute(req, decision, ctx);
        }
        let rid = req.request_id.as_str();
        // Only an effect AIEN's EffectLane minted for this request opens the ledger path. The
        // in-process provider never runs it: the effect runs through the daemon instead.
        let minted = self.shared.with(|a| a.holds_minted_effect(rid));
        self.shared.with(|a| a.discard_unexecuted(rid));
        let approver = self
            .state
            .borrow()
            .approvers
            .get(rid)
            .cloned()
            .unwrap_or_else(|| DEFAULT_APPROVER.into());
        let approval_id = decision
            .approval
            .as_ref()
            .map(|a| a.approval_id.clone())
            .unwrap_or_default();
        let out = if minted {
            self.write_through_ledger(req, &approver, &approval_id)
        } else {
            Err(format!(
                "{}: no AIEN-minted effect for this request",
                crate::NOT_EXECUTED
            ))
        };
        let mut r = match out {
            Ok(rc) => ToolResult::ok(rid, json!({"receipt": rc})),
            Err(e) => ToolResult::failed(
                Some(rid),
                ResultStatus::Error,
                ErrorCode::ExecutionError,
                &e,
            ),
        };
        let ok = r.status == ResultStatus::Ok;
        r.provenance = Some(ResultProvenance {
            runtime: Some(RUNTIME_ID.into()),
            capability: Some(req.capability.clone()),
            duration_ms: Some(0),
            content_kind: ok.then_some(ContentKind::ToolResult),
            trust: ok.then_some(TrustLevel::TrustedRuntime),
            trusted: None,
            extensions: Map::new(),
        });
        r
    }

    fn catalog(&self) -> Catalog {
        self.shared.catalog()
    }
}
