//! Crossveil: the authority boundary. A runtime adapter supplies `decide` and `execute`; the
//! [`Pipeline`] enforces the CORE.md / CROSSVEIL.md invariants around them and never fabricates,
//! caches, upgrades or reuses a decision.
use interplane_core::{CapabilityRequest, Catalog, Decision, Party, ToolResult};
use serde_json::Value;

pub mod mock;
pub mod pipeline;

pub use mock::{mock_mapping_table, mock_mapping_table_stale, mock_table_by_name, MockRuntime};
pub use pipeline::{ObservedRecord, Pipeline, TurnOutcome};

/// What the pipeline passes through, untouched, to the runtime callbacks.
#[derive(Debug, Clone)]
pub struct CallContext {
    pub trace_id: String,
    pub message_id: String,
    pub parent_id: Option<String>,
    pub model: Party,
    /// Runtime-local session handle the adapter may attach.
    pub session: Option<Value>,
}

/// The two callbacks that touch a runtime, plus its catalog.
pub trait RuntimeAuthority {
    fn runtime_id(&self) -> &str;
    /// The runtime's authority decision. Called for every mapped request.
    fn decide(&mut self, req: &CapabilityRequest, ctx: &CallContext) -> Decision;
    /// Called only after `decide` returned `authorized` for this exact request.
    fn execute(
        &mut self,
        req: &CapabilityRequest,
        decision: &Decision,
        ctx: &CallContext,
    ) -> ToolResult;
    fn catalog(&self) -> Catalog;
}
