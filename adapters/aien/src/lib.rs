//! INTERPLANE reference adapter for AIEN (ADR 0003).
//!
//! The adapter composes AIEN's existing primitives and mints nothing. It never constructs an
//! `AuthorizedEffect`, a `DoctrineDecision` or a `SafetyDecision`. `authorized` is only ever the
//! translation of an `Ok` that AIEN's own gate returned, for a capability AIEN classifies as
//! speculation-safe.
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aien_capability::{
    catalog_digest, routing_class, speculation_safe, Digest32, EffectClass, ProviderId,
    ToolDescriptor, ToolEffects,
};
use aien_mcp::memory::MemoryWire;
use aien_mcp::{CallOutcome, SessionManager, SpeculativeLane, SpeculativeToolCall};
use interplane_core::*;
use interplane_crossaxis::MappingTable;
use interplane_crossveil::{CallContext, RuntimeAuthority};
use serde_json::{json, Map, Value};

/// Runtime id carried in every decision, result and catalog.
pub const RUNTIME_ID: &str = "aien";
/// Pinned reply for any effectful capability (ADR 0003 item 2).
pub const APPROVAL_REASON: &str =
    "AIEN production cannot mint AuthorizedEffect; approval must come from AIEN";
/// Pinned reply for everything the reference adapter does not run.
pub const NOT_EXECUTED: &str = "not executed by the reference adapter";
/// Pinned reply when the AIEN gate is not linked or the adapter was built unavailable.
pub const UNAVAILABLE: &str = "aien runtime not available";

const MAX_READ_BYTES: u64 = 1 << 20;

/// One capability: AIEN-side name, effects, and the argument schema (all string-typed).
struct Spec {
    name: &'static str,
    effects: fn() -> ToolEffects,
    required: &'static [&'static str],
}

/// Names come from aegis `ALLOWED_SKILLS` (enforcement.rs:14-24). AIEN enrolls no catalog today
/// (CURRENT_STATE.md: aien-capability has no dependents), so the effect bits below are this
/// adapter's enrollment, using AIEN's own `ToolEffects`. They are inputs to AIEN's classifier,
/// not a classifier of ours.
const SPECS: [Spec; 4] = [
    Spec {
        name: "read_file",
        effects: || ToolEffects::READ_FILESYSTEM,
        required: &["path"],
    },
    Spec {
        name: "list_dir",
        effects: || ToolEffects::READ_FILESYSTEM,
        required: &["path"],
    },
    Spec {
        name: "write_file",
        effects: || ToolEffects::WORLD_MUTATION,
        required: &["path", "content"],
    },
    Spec {
        name: "bash_eval",
        effects: || ToolEffects::SPAWN_PROCESS | ToolEffects::WORLD_MUTATION,
        required: &["command"],
    },
];

fn schema_for(spec: &Spec) -> Map<String, Value> {
    let props: Map<String, Value> = spec
        .required
        .iter()
        .map(|k| (k.to_string(), json!({"type": "string"})))
        .collect();
    json!({"type": "object", "properties": props, "required": spec.required})
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn hex(d: &Digest32) -> String {
    d.0.iter().map(|b| format!("{b:02x}")).collect()
}

fn descriptors() -> Vec<ToolDescriptor> {
    SPECS
        .iter()
        .map(|s| {
            let sd = interplane_core::digest(&Value::Object(schema_for(s)));
            let raw = sd.trim_start_matches("sha256:");
            let mut bytes = [0u8; 32];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap_or(0);
            }
            ToolDescriptor::new(s.name, (s.effects)(), Digest32(bytes))
        })
        .collect()
}

/// The reference adapter. See the crate README for what is reused and what is reimplemented.
pub struct AienAuthority {
    workspace: Option<PathBuf>,
    trust_workspace: bool,
    available: bool,
    descriptors: Vec<ToolDescriptor>,
    lane: SpeculativeLane,
    provider: ProviderId,
    runtime: tokio::runtime::Runtime,
    wire_calls: Arc<AtomicUsize>,
    pub decide_calls: u32,
    pub execute_calls: u32,
}

impl AienAuthority {
    /// Enroll the in-process read provider over `workspace` and return the adapter.
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self, String> {
        let root = fs::canonicalize(workspace.as_ref()).map_err(|e| format!("workspace: {e}"))?;
        let mut a = Self::build(Some(root.clone()), true)?;
        a.enroll(root)?;
        Ok(a)
    }

    /// Fail-closed adapter: `decide()` denies everything with "aien runtime not available".
    /// This is also the behaviour when the crate is built without the `aegis-gate` feature.
    pub fn unavailable() -> Self {
        Self::build(None, false).expect("tokio current-thread runtime")
    }

    /// Mark workspace reads as `trusted_runtime` instead of `workspace_untrusted`.
    pub fn with_trusted_workspace(mut self, trusted: bool) -> Self {
        self.trust_workspace = trusted;
        self
    }

    /// Number of times the wire (the code that touches the filesystem) was invoked.
    pub fn wire_calls(&self) -> usize {
        self.wire_calls.load(Ordering::SeqCst)
    }

    fn build(workspace: Option<PathBuf>, available: bool) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            workspace,
            trust_workspace: false,
            available: available && cfg!(feature = "aegis-gate"),
            descriptors: descriptors(),
            lane: SpeculativeLane::new(aien_mcp::McpBroker::new()),
            provider: ProviderId::new("interplane-reference"),
            runtime,
            wire_calls: Arc::new(AtomicUsize::new(0)),
            decide_calls: 0,
            execute_calls: 0,
        })
    }

    fn enroll(&mut self, root: PathBuf) -> Result<(), String> {
        let counter = self.wire_calls.clone();
        let wire = MemoryWire::new(self.descriptors.clone(), move |name, args| {
            counter.fetch_add(1, Ordering::SeqCst);
            read_handler(&root, name, args)
        });
        let mgr = SessionManager::new();
        self.runtime
            .block_on(mgr.enroll(self.provider.clone(), Arc::new(wire)))
            .map_err(|e| e.to_string())?;
        self.lane = mgr.speculative_lane();
        Ok(())
    }

    fn spec(&self, name: &str) -> Option<(&Spec, &ToolDescriptor)> {
        let s = SPECS.iter().find(|s| s.name == name)?;
        let d = self.descriptors.iter().find(|d| d.name() == name)?;
        Some((s, d))
    }

    /// The mapping table for this adapter: pass-through of every AIEN capability name.
    /// `pinned` pins the table to the live catalog digest so a changed catalog is `stale_capability`.
    pub fn mapping_table(&self, pinned: bool) -> MappingTable {
        let rules: Vec<Value> = SPECS
            .iter()
            .map(|s| {
                json!({"id": format!("passthrough:{}", s.name), "kind": "passthrough",
                    "from": {"namespace": null, "name": s.name}, "to": s.name})
            })
            .collect();
        let pin = pinned.then(|| self.catalog().catalog_digest);
        MappingTable::from_value(json!({"runtime": RUNTIME_ID, "table_version": "1",
            "catalog_digest": pin.flatten(), "rules": rules}))
        .expect("static table")
    }

    fn decision(
        &self,
        req: &CapabilityRequest,
        kind: DecisionKind,
        reason: Option<&str>,
        engine: &str,
    ) -> Decision {
        Decision {
            kind: DecisionMsgKind,
            request_id: req.request_id.clone(),
            decision: kind,
            capability: Some(req.capability.clone()),
            authority: Authority {
                runtime: RUNTIME_ID.into(),
                policy_engine: engine.into(),
                decision_id: None,
                extensions: Map::new(),
            },
            reason: reason.map(str::to_string),
            constraints: vec![],
            approval: None,
            runtime_state: None,
            extensions: Map::new(),
        }
    }

    fn confine(&self, path: &str) -> Result<(), String> {
        let root = self.workspace.as_ref().ok_or(UNAVAILABLE)?;
        let p = Path::new(path);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        let inside = match fs::canonicalize(&full) {
            Ok(c) => c.starts_with(root),
            Err(_) => {
                full.starts_with(root) && !full.components().any(|c| c == Component::ParentDir)
            }
        };
        if inside {
            Ok(())
        } else {
            Err(format!("path is outside the workspace: {path}"))
        }
    }
}

/// Reimplemented: the in-process provider body (AIEN ships `MemoryWire` but no filesystem tools
/// in aien-mcp). Runs only behind `SpeculativeLane::invoke_speculative`.
fn read_handler(root: &Path, name: &str, args: &Value) -> CallOutcome {
    let Some(path) = args.get("path").and_then(Value::as_str) else {
        return CallOutcome::Rejected("missing required argument: path".into());
    };
    let p = Path::new(path);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    let Ok(real) = fs::canonicalize(&full) else {
        return CallOutcome::Rejected(format!("cannot read {path}: no such file or directory"));
    };
    if !real.starts_with(root) {
        return CallOutcome::Rejected(format!("path is outside the workspace: {path}"));
    }
    match name {
        "read_file" => match fs::metadata(&real) {
            Ok(m) if m.is_file() && m.len() <= MAX_READ_BYTES => match fs::read_to_string(&real) {
                Ok(s) => CallOutcome::Finished(json!({"path": path, "content": s})),
                Err(e) => CallOutcome::Rejected(format!("cannot read {path}: {e}")),
            },
            Ok(_) => CallOutcome::Rejected(format!("cannot read {path}: not a readable file")),
            Err(e) => CallOutcome::Rejected(format!("cannot read {path}: {e}")),
        },
        "list_dir" => match fs::read_dir(&real) {
            Ok(rd) => {
                let mut names: Vec<String> = rd
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                CallOutcome::Finished(json!({"path": path, "entries": names}))
            }
            Err(e) => CallOutcome::Rejected(format!("cannot list {path}: {e}")),
        },
        _ => CallOutcome::Rejected(NOT_EXECUTED.into()),
    }
}

fn type_ok(v: &Value) -> bool {
    v.is_string()
}

impl RuntimeAuthority for AienAuthority {
    fn runtime_id(&self) -> &str {
        RUNTIME_ID
    }

    fn decide(&mut self, req: &CapabilityRequest, _ctx: &CallContext) -> Decision {
        self.decide_calls += 1;
        if !self.available {
            return self.decision(
                req,
                DecisionKind::Denied,
                Some(UNAVAILABLE),
                "adapter.fail_closed",
            );
        }
        let Some((spec, desc)) = self.spec(&req.capability) else {
            let r = format!("unknown capability: {}", req.capability);
            return self.decision(
                req,
                DecisionKind::NotFound,
                Some(&r),
                "aien-capability.catalog",
            );
        };
        for key in spec.required {
            match req.arguments.get(*key) {
                None => {
                    let r = format!("missing required argument: {key}");
                    return self.decision(
                        req,
                        DecisionKind::Invalid,
                        Some(&r),
                        "aien-capability.catalog",
                    );
                }
                Some(v) if !type_ok(v) => {
                    let r = format!("argument {key} must be string");
                    return self.decision(
                        req,
                        DecisionKind::Invalid,
                        Some(&r),
                        "aien-capability.catalog",
                    );
                }
                Some(_) => {}
            }
        }
        // AIEN's live gate, for every known capability.
        #[cfg(feature = "aegis-gate")]
        if let Err(reason) =
            aegis::pre_dispatch_check(&req.capability, &Value::Object(req.arguments.clone()))
        {
            return self.decision(
                req,
                DecisionKind::Denied,
                Some(&reason),
                "aegis.enforcement.pre_dispatch_check",
            );
        }
        if !speculation_safe(desc.effects()) {
            let mut d = self.decision(
                req,
                DecisionKind::RequiresApproval,
                Some(APPROVAL_REASON),
                "aien-mcp.speculation_safe",
            );
            let mut values = vec![format!("effect_class={:?}", routing_class(desc.effects()))];
            // AIEN records the intent without executing it. No approval id exists until AIEN gives one.
            let staged = self.runtime.block_on(self.lane.stage_effect_intent(
                &self.provider,
                &req.capability,
                Value::Object(req.arguments.clone()),
            ));
            match staged {
                Ok(intent) => {
                    values.push("staged=true".into());
                    values.push(format!(
                        "capability_digest={}",
                        hex(&intent.capability_digest)
                    ));
                }
                Err(e) => values.push(format!("staged=false: {e}")),
            }
            d.runtime_state = Some(RuntimeExtension {
                vocabulary: "aien.effect_intent".into(),
                values,
                extensions: Map::new(),
            });
            return d;
        }
        // Adapter restriction (can only deny): confine reads to the workspace root.
        if let Some(path) = req.arguments.get("path").and_then(Value::as_str) {
            if let Err(why) = self.confine(path) {
                return self.decision(
                    req,
                    DecisionKind::Denied,
                    Some(&why),
                    "adapter.workspace_confinement",
                );
            }
        }
        let mut d = self.decision(
            req,
            DecisionKind::Authorized,
            None,
            "aegis.enforcement.pre_dispatch_check",
        );
        d.runtime_state = Some(RuntimeExtension {
            vocabulary: "aien.effects".into(),
            values: vec![format!("effect_class={:?}", routing_class(desc.effects()))],
            extensions: Map::new(),
        });
        d
    }

    fn execute(
        &mut self,
        req: &CapabilityRequest,
        decision: &Decision,
        _ctx: &CallContext,
    ) -> ToolResult {
        self.execute_calls += 1;
        let rid = req.request_id.as_str();
        let fail = |msg: &str| {
            ToolResult::failed(
                Some(rid),
                ResultStatus::Error,
                ErrorCode::ExecutionError,
                msg,
            )
        };
        let readable = self
            .spec(&req.capability)
            .map(|(_, d)| routing_class(d.effects()) == EffectClass::ReadOnly)
            .unwrap_or(false);
        let mut r = if !decision.is_authorized() || !readable {
            fail(NOT_EXECUTED)
        } else {
            let call = SpeculativeToolCall {
                provider: self.provider.clone(),
                tool_name: req.capability.clone(),
                arguments: Value::Object(req.arguments.clone()),
                sandboxed: false,
            };
            match self.runtime.block_on(self.lane.invoke_speculative(call)) {
                Ok(out) => ToolResult::ok(rid, out.output),
                Err(e) => fail(&match e {
                    aien_mcp::Error::Rejected(why) => why,
                    other => other.to_string(),
                }),
            }
        };
        let ok = r.status == ResultStatus::Ok;
        let (kind, trust) = if ok {
            let t = if self.trust_workspace {
                TrustLevel::TrustedRuntime
            } else {
                TrustLevel::WorkspaceUntrusted
            };
            (Some(ContentKind::WorkspaceContent), Some(t))
        } else {
            (None, None)
        };
        r.provenance = Some(ResultProvenance {
            runtime: Some(RUNTIME_ID.into()),
            capability: Some(req.capability.clone()),
            duration_ms: Some(0),
            content_kind: kind,
            trust,
            trusted: None,
            extensions: Map::new(),
        });
        r
    }

    fn catalog(&self) -> Catalog {
        let capabilities = SPECS
            .iter()
            .zip(self.descriptors.iter())
            .map(|(s, d)| CapabilityDescriptor {
                name: s.name.into(),
                canonical: None,
                description: format!("AIEN capability {}", s.name),
                parameters: schema_for(s),
                domains: vec!["filesystem".into()],
                runtime_effects: Some(RuntimeExtension {
                    vocabulary: "aien.tool_effects".into(),
                    values: vec![
                        format!("bits={:#x}", d.effects().bits()),
                        format!("class={:?}", routing_class(d.effects())),
                        format!("speculation_safe={}", speculation_safe(d.effects())),
                    ],
                    extensions: Map::new(),
                }),
                schema_digest: None,
                extensions: Map::new(),
            })
            .collect();
        let aien = catalog_digest(&self.descriptors);
        let mut c = Catalog {
            kind: CatalogKind,
            runtime: RUNTIME_ID.into(),
            catalog_version: "1".into(),
            catalog_digest: Some(format!("sha256:{}", hex(&aien))),
            capabilities,
            extensions: Map::new(),
        };
        let computed = c.compute_digest();
        let same = c.catalog_digest.as_deref() == Some(computed.as_str());
        c.extensions.insert(
            "aien".into(),
            json!({
                "catalog_digest": format!("sha256:{}", hex(&aien)),
                "interplane_computed_digest": computed,
                "digests_equal": same,
                "formula": "aien_capability::catalog_digest: sha256 over concatenated per-tool digests sorted by name; per-tool digest = sha256(name || effects_bits_le32 || schema_digest)",
            }),
        );
        c
    }
}
