//! `write_file` through AIEN's production compose path and durable effect ledger (`aien-cli
//! daemon`, sovereign-core #249).
//!
//! [`ComposeLedgerAuthority`] wraps [`AienShared`]: the approval desk, `decide` and every read are
//! the tested in-process adapter, unchanged. Only an authorized `write_file` takes a different
//! route, after the host approved it through the host-only continuation:
//!
//! 1. the authenticated handoff: an `ApprovedProposal` (trace id, request id, approval id,
//!    approver, path, content, approved proposal digest, content digest) with `approval_mac`, the
//!    HMAC-SHA256 under the approval desk key ([`DeskKey`]) over the canonical binding, sent as
//!    `ComposeApprovedProposal`. The daemon re-checks the MAC, claims the request id, approval id
//!    and approval key durably (replay refused across restarts), then runs the approved text
//!    through compose: J-Space branch, AEGIS verify callback, World commit, Cortex records;
//! 2. the returned `ApprovedComposeReport` is checked field by field (ids, both hash identities);
//! 3. the grant is the daemon's own reserved `approved_grant` record (returned as
//!    `approved_grant`): keyed on `compose_proposal_sha256` (never the pre-compose approved
//!    digest), confined to the workspace, linked to the Cortex promotion, evidence and replay claim
//!    records; it is read back and checked. This authority writes no grant itself, so an approved
//!    write has exactly one route;
//! 4. `ComposeEffectIntent` (stop, revoke, spent, stale checked by the daemon), the write (tmp +
//!    rename inside the workspace), `ComposeEffectAck` (the daemon reads the disk).
//!
//! The desk key never reaches the model: it is read from a host-configured path outside the
//! workspace ([`ComposeLedgerAuthority::new`] refuses a key inside it) and only its id is recorded.
//! The socket client speaks the daemon's JSON line protocol directly (`aien-runtime` is not a
//! dependency: its build scripts need the omega libraries). Boundary: [`LEDGER_BOUNDARY`].
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
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
pub const LEDGER_BOUNDARY: &str = "boundary = authenticated daemon handoff (ComposeApprovedProposal: desk MAC, durable replay claim) -> compose verify + AEGIS -> J-Space branch -> World commit -> Cortex promotion/evidence -> daemon-written grant on compose_proposal_sha256 -> effect intent -> write -> ack; the production effect path, not WALDO provenance";
/// Version tag inside every approval binding (sovereign-core `approved_auth`).
pub const APPROVAL_BINDING_VERSION: &str = "aien.approval.v2";
/// Approver recorded in the grant when the host names none.
pub const DEFAULT_APPROVER: &str = "interplane-host";

/// sha256 hex (the daemon's digest form: lowercase, no prefix).
pub fn sha256_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// HMAC-SHA256 (RFC 2104) over sha2.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new()
        .chain_update(ipad)
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(opad)
        .chain_update(inner)
        .finalize()
        .into()
}

/// The fields one approval covers, in the canonical binding the daemon re-computes (compact
/// JSON, keys sorted, plus `"v": APPROVAL_BINDING_VERSION`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalBinding {
    pub trace_id: String,
    pub request_id: String,
    pub approval_id: String,
    pub approver: String,
    pub path: String,
    pub content_sha256: String,
    pub approved_proposal_sha256: String,
    pub desk_key_id: String,
    /// The canonical absolute workspace the effect may land in (the daemon binds the
    /// canonicalised workspace of the command, so another workspace fails the MAC).
    pub workspace: String,
}

impl ApprovalBinding {
    pub fn bytes(&self) -> Vec<u8> {
        let mut m = Map::new();
        for (k, v) in [
            ("approval_id", &self.approval_id),
            ("approved_proposal_sha256", &self.approved_proposal_sha256),
            ("approver", &self.approver),
            ("content_sha256", &self.content_sha256),
            ("desk_key_id", &self.desk_key_id),
            ("path", &self.path),
            ("request_id", &self.request_id),
            ("trace_id", &self.trace_id),
            ("workspace", &self.workspace),
        ] {
            m.insert(k.into(), json!(v));
        }
        m.insert("v".into(), json!(APPROVAL_BINDING_VERSION));
        Value::Object(m).to_string().into_bytes()
    }
}

/// The approval desk key (`<compose dir>/approval-desk.key`, made by `aien compose desk-key
/// --create 1`). Loaded with the daemon's rules: a regular file (no symlink, `O_NOFOLLOW`) owned
/// by this user, no group or other bits, 64 lowercase hex digits. `Debug` shows only the id.
pub struct DeskKey {
    key: [u8; 32],
    id: String,
}

impl std::fmt::Debug for DeskKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeskKey {{ id: {} }}", self.id)
    }
}

impl DeskKey {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bad = |w: String| format!("approval desk key {}: {w}", path.display());
        let l = fs::symlink_metadata(path).map_err(|e| bad(e.to_string()))?;
        if l.file_type().is_symlink() {
            return Err(bad("is a symlink".into()));
        }
        let f = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc_o_nofollow())
            .open(path)
            .map_err(|e| bad(e.to_string()))?;
        let m = f.metadata().map_err(|e| bad(e.to_string()))?;
        let me = fs::metadata("/proc/self")
            .map_err(|e| bad(e.to_string()))?
            .uid();
        if !m.file_type().is_file() || m.uid() != me || m.mode() & 0o077 != 0 {
            return Err(bad(
                "must be a regular file owned by this user with mode 0600".into(),
            ));
        }
        let mut s = String::new();
        f.take(256)
            .read_to_string(&mut s)
            .map_err(|e| bad(e.to_string()))?;
        let h = s.trim_end_matches('\n');
        if h.len() != 64
            || !h
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(bad("not 64 lowercase hex digits".into()));
        }
        let mut key = [0u8; 32];
        for (i, o) in key.iter_mut().enumerate() {
            *o = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).map_err(|e| bad(e.to_string()))?;
        }
        let id = sha256_hex(&key)[..16].to_string();
        Ok(Self { key, id })
    }

    /// First 16 hex digits of sha256 of the key.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `approval_mac` for `b` (whose `desk_key_id` must be [`Self::id`]).
    pub fn mac(&self, b: &ApprovalBinding) -> String {
        hmac_sha256(&self.key, &b.bytes())
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    }
}

impl Drop for DeskKey {
    fn drop(&mut self) {
        for b in self.key.iter_mut() {
            // SAFETY: a valid, aligned, exclusively borrowed byte.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

/// `O_NOFOLLOW` on Linux (aarch64 and x86_64 differ; the adapter is Linux-only like the daemon).
fn libc_o_nofollow() -> i32 {
    if cfg!(target_arch = "aarch64") {
        0o100000
    } else {
        0o400000
    }
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

    /// `ComposeApprovedProposal` (sovereign-core #249). `Ok` = the `ComposeApprovedResult` report;
    /// a `ComposeApprovedRefused` is `Err` with `"<refused_by> <name>: <detail>"`.
    pub fn approved(&self, proposal: &Value, workspace: &str) -> Result<Value, String> {
        let v = self.send(
            json!({"ComposeApprovedProposal": {"proposal": proposal, "workspace": workspace}}),
        )?;
        if let Some(r) = v.get("ComposeApprovedRefused") {
            return Err(format!(
                "{} {}: {}",
                r["refused_by"].as_str().unwrap_or("REFUSED"),
                r["name"].as_str().unwrap_or("?"),
                r["detail"].as_str().unwrap_or("")
            ));
        }
        v.get("ComposeApprovedResult")
            .cloned()
            .ok_or_else(|| format!("expected ComposeApprovedResult, got {v}"))
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
    desk_key: PathBuf,
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
    /// `shared` must be enrolled over the same `workspace`; `socket` is `$AIEN_RUNTIME_SOCK`;
    /// `desk_key` is the daemon's approval desk key (`aien compose desk-key`). Refused when the key
    /// fails the file rules or sits inside the workspace (the model-facing tools read there).
    pub fn new(
        shared: AienShared,
        workspace: impl AsRef<Path>,
        socket: impl Into<PathBuf>,
        desk_key: impl AsRef<Path>,
    ) -> Result<Self, String> {
        let workspace =
            fs::canonicalize(workspace.as_ref()).map_err(|e| format!("workspace: {e}"))?;
        let desk_key = fs::canonicalize(desk_key.as_ref().parent().unwrap_or(Path::new("/")))
            .map_err(|e| format!("approval desk key: {e}"))?
            .join(
                desk_key
                    .as_ref()
                    .file_name()
                    .ok_or("approval desk key: no file name")?,
            );
        if desk_key.starts_with(&workspace)
            || workspace.starts_with(desk_key.parent().unwrap_or(Path::new("/")))
        {
            return Err(format!(
                "approval desk key {} and workspace {} overlap; refusing",
                desk_key.display(),
                workspace.display()
            ));
        }
        DeskKey::load(&desk_key)?;
        Ok(Self {
            shared,
            state: Rc::new(RefCell::new(LedgerState {
                client: LedgerClient::new(socket),
                workspace,
                desk_key,
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

    /// Daemon-written grants (reserved `approved_grant` records) this authority received for
    /// approvals it handed off and checked; it never writes a grant itself.
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

    /// handoff -> grant -> intent -> atomic write -> ack. `Err` before the intent means nothing
    /// was written.
    fn write_through_ledger(
        &self,
        req: &CapabilityRequest,
        trace_id: &str,
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
        let ws = self
            .state
            .borrow()
            .workspace
            .to_str()
            .ok_or("workspace is not UTF-8")?
            .to_string();
        // approved_proposal_sha256: the approved {content, path} object, keys sorted.
        let mut prop = Map::new();
        prop.insert("content".into(), json!(content));
        prop.insert("path".into(), json!(path));
        let approved_sha = sha256_hex(Value::Object(prop).to_string().as_bytes());
        let content_sha256 = sha256_hex(content.as_bytes());
        let rid = req.request_id.as_str();
        if approval_id.is_empty() {
            return Err("no approval id for this request".into());
        }
        // 1. The authenticated handoff.
        let desk = DeskKey::load(&self.state.borrow().desk_key)?;
        let binding = ApprovalBinding {
            trace_id: trace_id.into(),
            request_id: rid.into(),
            approval_id: approval_id.into(),
            approver: approver.into(),
            path: path.into(),
            content_sha256: content_sha256.clone(),
            approved_proposal_sha256: approved_sha.clone(),
            desk_key_id: desk.id().into(),
            workspace: ws.clone(),
        };
        let proposal = json!({"request_id": rid, "trace_id": trace_id, "approval_id": approval_id,
            "approver": approver, "path": path, "content": content,
            "approved_proposal_sha256": approved_sha, "content_sha256": content_sha256,
            "approval_mac": desk.mac(&binding)});
        drop(desk);
        let client = self.client();
        let mut rc = json!({"boundary": LEDGER_BOUNDARY, "trace_id": trace_id, "request_id": rid,
            "approval_id": approval_id, "approver": approver, "path": path, "target": tgt,
            "approved_proposal_sha256": approved_sha, "content_sha256": content_sha256});
        let report = match client.approved(&proposal, &ws) {
            Ok(r) => r,
            Err(e) => {
                rc["state"] = json!("REFUSED");
                rc["handoff_error"] = json!(e);
                self.state.borrow_mut().receipts.push(rc);
                return Err(e);
            }
        };
        // 2. Check what came back: ids unchanged, both hash identities, a fresh commit.
        let compose_sha = sha256_hex(format!("filename: {path}\n{content}").as_bytes());
        let field = |k: &str| report[k].as_str().unwrap_or_default().to_string();
        let checks = [
            ("state", field("state"), "COMMITTED".to_string()),
            ("trace_id", field("trace_id"), trace_id.to_string()),
            ("request_id", field("request_id"), rid.to_string()),
            ("approval_id", field("approval_id"), approval_id.to_string()),
            (
                "approved_proposal_sha256",
                field("approved_proposal_sha256"),
                approved_sha.clone(),
            ),
            (
                "content_sha256",
                field("content_sha256"),
                content_sha256.clone(),
            ),
            (
                "compose_proposal_sha256",
                field("compose_proposal_sha256"),
                compose_sha.clone(),
            ),
        ];
        let task = &report["task"];
        let links: Vec<u64> = report["grant_links"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let claim = report["replay_claim"].as_u64().unwrap_or(0);
        rc["compose_proposal_sha256"] = json!(compose_sha);
        rc["handoff"] = json!({"state": report["state"], "desk_key_id": report["desk_key_id"],
            "approved_grant": report["approved_grant"], "target": report["target"],
            "approval_key": report["approval_key"], "replay_claim": claim,
            "task": task["task"], "outcome": task["outcome"], "committed": task["committed"],
            "branch_count": task["branch_count"], "winner": task["winner"],
            "aegis_pass_mask": task["aegis_pass_mask"], "proposer": task["proposer"],
            "cx_goal": task["cx_goal"], "cx_evidence": task["cx_evidence"],
            "cx_promotion": task["cx_promotion"], "winner_digest": task["winner_digest"],
            "record_digest": task["record_digest"], "grant_links": links});
        let bad: Vec<String> = checks
            .iter()
            .filter(|(_, got, want)| got != want)
            .map(|(k, got, want)| format!("{k}: daemon {got:?}, expected {want:?}"))
            .collect();
        let committed = task["committed"] == json!(true)
            && task["proposal_sha256"].as_str() == Some(compose_sha.as_str())
            && links.len() == 2
            && links.iter().all(|&x| x != 0)
            && claim != 0;
        if !bad.is_empty() || !committed {
            let e = format!(
                "handoff report does not match the approval ({}{})",
                bad.join("; "),
                if committed {
                    ""
                } else {
                    "; no fresh committed compose run"
                }
            );
            rc["state"] = json!("REFUSED");
            rc["handoff_error"] = json!(e);
            self.state.borrow_mut().receipts.push(rc);
            return Err(e);
        }
        // 3. The grant: written by the daemon itself after the committed compose (a reserved
        // `approved_grant` record keyed on compose_proposal_sha256, linked to promotion, evidence
        // and the replay claim). This authority never writes its own grant: there is no second
        // route to an effect.
        let grant_id = report["approved_grant"].as_u64().unwrap_or(0);
        let g = client
            .recall(&[grant_id], None)
            .ok()
            .and_then(|r| r["cited"].as_array().and_then(|a| a.first().cloned()))
            .unwrap_or(Value::Null);
        let gt: Value = g["text"]
            .as_str()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or(Value::Null);
        let glinks: Vec<u64> = g["links"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let grant_ok = grant_id != 0
            && g["verified"] == json!(true)
            && g["note"] == json!("authorization")
            && gt["approved_grant"] == json!(1)
            && gt["proposal_sha256"].as_str() == Some(compose_sha.as_str())
            && gt["path"].as_str() == Some(path)
            && gt["content_sha256"].as_str() == Some(content_sha256.as_str())
            && gt["target"].as_str() == Some(tgt.as_str())
            && report["target"].as_str() == Some(tgt.as_str())
            && gt["replay_claim"].as_u64() == Some(claim)
            && [links[0], links[1], claim]
                .iter()
                .all(|x| glinks.contains(x));
        if !grant_ok {
            let e = format!("daemon grant #{grant_id} does not match the committed approval: {g}");
            rc["state"] = json!("REFUSED");
            rc["handoff_error"] = json!(e);
            self.state.borrow_mut().receipts.push(rc);
            return Err(e);
        }
        self.state.borrow_mut().grants_minted += 1;
        let prior = gt["prior_sha256"].clone();
        let hook = self.state.borrow_mut().before_intent.take();
        if let Some(mut h) = hook {
            h(&target);
            self.state.borrow_mut().before_intent = Some(h);
        }
        rc["prior_sha256"] = prior;
        rc["grant_id"] = json!(grant_id);
        rc["grant_digest"] = g["digest"].clone();
        rc["proposal_sha256"] = json!(compose_sha);
        // 4. Intent (on compose_proposal_sha256), write, ack.
        let intent = match client.intent(
            grant_id,
            &compose_sha,
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
        let written = atomic_write(Path::new(&ws), &target, content.as_bytes());
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

/// The parent of `target` must still resolve (symlinks followed) to itself, inside `workspace`
/// (canonical). Checked before the temp file is created and again just before the rename.
fn parent_confined(workspace: &Path, target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("write {}: no parent directory", target.display()))?;
    let real =
        fs::canonicalize(parent).map_err(|e| format!("write {}: parent: {e}", target.display()))?;
    if real != parent || !real.starts_with(workspace) {
        return Err(format!(
            "write {}: refused, parent {} resolves to {} (outside the workspace or through a symlink)",
            target.display(),
            parent.display(),
            real.display()
        ));
    }
    Ok(real)
}

/// A random hex suffix for the temp file name (no new dependency: /dev/urandom).
fn random_suffix() -> Result<String, String> {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("random temp name: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write `bytes` to `target` through a temp file in the same, confined directory: the temp file
/// gets an unpredictable name and is opened `create_new` + `O_NOFOLLOW`, mode 0600, so a symlink
/// planted at a guessed name is never followed (476ca4 x4); the parent is re-checked inside the
/// workspace just before the rename, which is refused otherwise.
fn atomic_write(workspace: &Path, target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = parent_confined(workspace, target)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.interplane-{}.tmp", random_suffix()?));
    let r = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc_o_nofollow())
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()))
        .map_err(|e| format!("write {}: temp file: {e}", target.display()))
        .and_then(|_| parent_confined(workspace, target).map(|_| ()))
        .and_then(|_| {
            fs::rename(&tmp, target).map_err(|e| format!("write {}: {e}", target.display()))
        });
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
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
        // in-process provider never runs it: the effect runs through the daemon instead. The
        // route holds the effect (and its grant reservation, sovereign-core #260) until the daemon
        // answers; the grant's expiry is checked with the adapter's clock just before the handoff
        // (the commit point of this route). The daemon's durable replay ledger is what spends the
        // approval; afterwards AIEN's reservation is released with that reason.
        let effect = self.shared.with(|a| {
            a.discard_replayed(rid);
            a.take_minted(rid)
        });
        let expiry = match &effect {
            Some(e) => self.shared.with(|a| a.check_expiry(e)),
            None => Ok(()),
        };
        let minted = effect.is_some();
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
        let out = if let Err(e) = expiry {
            Err(e)
        } else if minted {
            self.write_through_ledger(req, &ctx.trace_id, &approver, &approval_id)
        } else {
            Err(format!(
                "{}: no AIEN-minted effect for this request",
                crate::NOT_EXECUTED
            ))
        };
        if let Some(e) = effect {
            e.cancel(match &out {
                Ok(_) => "interplane ledger route: the daemon's durable replay ledger spent this approval",
                Err(_) => "interplane ledger route: refused, nothing written by this route",
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 476ca4 x4 hardening: the write stays in a confined parent and never follows a symlink.
    #[test]
    fn atomic_write_stays_inside_the_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let ws = root.join("ws");
        let outdir = root.join("outdir");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outdir).unwrap();
        // Honest write: lands, mode of the result is the temp file's 0600, no temp left behind.
        atomic_write(&ws, &ws.join("a.txt"), b"one\n").unwrap();
        assert_eq!(fs::read(ws.join("a.txt")).unwrap(), b"one\n");
        assert_eq!(
            fs::read_dir(&ws).unwrap().count(),
            1,
            "temp file left behind"
        );
        // Parent directory is a symlink to outside: refused, nothing written outside.
        std::os::unix::fs::symlink(&outdir, ws.join("sub")).unwrap();
        let e = atomic_write(&ws, &ws.join("sub/b.txt"), b"two\n").unwrap_err();
        assert!(e.contains("refused"), "{e}");
        assert_eq!(fs::read_dir(&outdir).unwrap().count(), 0);
        // A parent outside the workspace altogether: refused.
        let e = atomic_write(&ws, &outdir.join("c.txt"), b"three\n").unwrap_err();
        assert!(e.contains("refused"), "{e}");
        assert_eq!(fs::read_dir(&outdir).unwrap().count(), 0);
        // Two temp names never repeat.
        assert_ne!(random_suffix().unwrap(), random_suffix().unwrap());
    }
}
