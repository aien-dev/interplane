//! The Crossveil lifecycle state machine (CORE.md). Authority only enters through a `&Decision`.
use crate::types::{Decision, DecisionKind};
use thiserror::Error;

/// Lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Proposed,
    Rejected,
    Mapped,
    Denied,
    RequiresApproval,
    Authorized,
    Executing,
    Succeeded,
    Failed,
    TimedOut,
}

impl State {
    /// Terminal states (REQUIRES_APPROVAL is terminal only for the model, so it is not listed).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Rejected | State::Denied | State::Succeeded | State::Failed | State::TimedOut
        )
    }
    /// The CORE.md stage name.
    pub fn name(self) -> &'static str {
        match self {
            State::Proposed => "PROPOSED",
            State::Rejected => "REJECTED",
            State::Mapped => "MAPPED",
            State::Denied => "DENIED",
            State::RequiresApproval => "REQUIRES_APPROVAL",
            State::Authorized => "AUTHORIZED",
            State::Executing => "EXECUTING",
            State::Succeeded => "SUCCEEDED",
            State::Failed => "FAILED",
            State::TimedOut => "TIMED_OUT",
        }
    }
}

/// A refused transition. The state does not change.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LifecycleError {
    #[error("illegal transition from {from:?}")]
    Illegal { from: State },
    #[error("decision is not authorized")]
    NotAuthorized,
    #[error("decision is for a different request")]
    WrongRequest,
    /// The continuation did not cite the approval_id the runtime minted for this request
    /// (absent, empty, never minted, or minted for another request).
    #[error("approval requires a new decision citing the approval_id")]
    MissingApproval,
}

/// One request's lifecycle. The state field is private and there is no setter, so an
/// `Authorized` or `Executing` state can only come from [`Lifecycle::apply_decision`] /
/// [`Lifecycle::begin_execution`] with an authorized [`Decision`].
#[derive(Debug, Clone)]
pub struct Lifecycle {
    request_id: String,
    state: State,
    /// The non-empty approval_id the runtime minted in the decision that entered
    /// REQUIRES_APPROVAL. A continuation must cite exactly this id.
    approval_id: Option<String>,
}

impl Lifecycle {
    /// A new lifecycle in `Proposed`.
    pub fn new(request_id: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            state: State::Proposed,
            approval_id: None,
        }
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// PROPOSED -> REJECTED (refused before runtime contact) or MAPPED -> REJECTED is via decision.
    pub fn reject(&mut self) -> Result<State, LifecycleError> {
        match self.state {
            State::Proposed => self.set(State::Rejected),
            from => Err(LifecycleError::Illegal { from }),
        }
    }
    /// PROPOSED -> MAPPED.
    pub fn map(&mut self) -> Result<State, LifecycleError> {
        match self.state {
            State::Proposed => self.set(State::Mapped),
            from => Err(LifecycleError::Illegal { from }),
        }
    }
    /// Apply a runtime decision. From MAPPED: authorized -> AUTHORIZED, denied (and any unknown
    /// value) -> DENIED, requires_approval -> REQUIRES_APPROVAL, invalid/not_found -> REJECTED.
    /// From REQUIRES_APPROVAL: only a NEW decision citing the non-empty `approval.approval_id`
    /// the runtime minted when it required approval; authorized -> AUTHORIZED, every other
    /// value -> DENIED. Any other id (absent, empty, never minted, minted for another request)
    /// is refused with [`LifecycleError::MissingApproval`] and the state does not change.
    pub fn apply_decision(&mut self, d: &Decision) -> Result<State, LifecycleError> {
        if d.request_id != self.request_id {
            return Err(LifecycleError::WrongRequest);
        }
        match self.state {
            State::Mapped => {
                let next = match &d.decision {
                    DecisionKind::Authorized => State::Authorized,
                    DecisionKind::Denied => State::Denied,
                    DecisionKind::RequiresApproval => {
                        self.approval_id = minted_id(d);
                        State::RequiresApproval
                    }
                    DecisionKind::NotFound | DecisionKind::Invalid => State::Rejected,
                    DecisionKind::Unknown(_) => State::Denied,
                };
                self.set(next)
            }
            State::RequiresApproval => {
                let cited = minted_id(d);
                if cited.is_none() || cited != self.approval_id {
                    return Err(LifecycleError::MissingApproval);
                }
                match &d.decision {
                    DecisionKind::Authorized => self.set(State::Authorized),
                    _ => self.set(State::Denied),
                }
            }
            from => Err(LifecycleError::Illegal { from }),
        }
    }
    /// REQUIRES_APPROVAL -> DENIED without a decision: a host cancel, or a continuation the
    /// pipeline refused to honour (fail closed). It can only deny, so it is the one way out of
    /// REQUIRES_APPROVAL that is not a runtime decision and still cannot authorize anything.
    pub fn cancel(&mut self) -> Result<State, LifecycleError> {
        match self.state {
            State::RequiresApproval => self.set(State::Denied),
            from => Err(LifecycleError::Illegal { from }),
        }
    }
    /// AUTHORIZED -> EXECUTING; needs the authorized decision itself.
    pub fn begin_execution(&mut self, d: &Decision) -> Result<State, LifecycleError> {
        if d.request_id != self.request_id {
            return Err(LifecycleError::WrongRequest);
        }
        if !d.is_authorized() {
            return Err(LifecycleError::NotAuthorized);
        }
        match self.state {
            State::Authorized => self.set(State::Executing),
            from => Err(LifecycleError::Illegal { from }),
        }
    }
    /// EXECUTING -> SUCCEEDED | FAILED | TIMED_OUT.
    pub fn finish(&mut self, outcome: State) -> Result<State, LifecycleError> {
        match (self.state, outcome) {
            (State::Executing, State::Succeeded | State::Failed | State::TimedOut) => {
                self.set(outcome)
            }
            (from, _) => Err(LifecycleError::Illegal { from }),
        }
    }
    fn set(&mut self, s: State) -> Result<State, LifecycleError> {
        self.state = s;
        Ok(s)
    }
}

/// The decision's `approval.approval_id`, if present and non-empty.
fn minted_id(d: &Decision) -> Option<String> {
    d.approval
        .as_ref()
        .map(|a| a.approval_id.clone())
        .filter(|id| !id.is_empty())
}
