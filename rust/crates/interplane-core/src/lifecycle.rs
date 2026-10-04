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
    #[error("approval transition needs a decision carrying approval.approval_id")]
    MissingApproval,
}

/// One request's lifecycle. The state field is private and there is no setter, so an
/// `Authorized` or `Executing` state can only come from [`Lifecycle::apply_decision`] /
/// [`Lifecycle::begin_execution`] with an authorized [`Decision`].
#[derive(Debug, Clone)]
pub struct Lifecycle {
    request_id: String,
    state: State,
}

impl Lifecycle {
    /// A new lifecycle in `Proposed`.
    pub fn new(request_id: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            state: State::Proposed,
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
    /// From REQUIRES_APPROVAL: only a NEW decision carrying `approval.approval_id`, either
    /// authorized -> AUTHORIZED or denied/unknown -> DENIED.
    pub fn apply_decision(&mut self, d: &Decision) -> Result<State, LifecycleError> {
        if d.request_id != self.request_id {
            return Err(LifecycleError::WrongRequest);
        }
        match self.state {
            State::Mapped => {
                let next = match &d.decision {
                    DecisionKind::Authorized => State::Authorized,
                    DecisionKind::Denied => State::Denied,
                    DecisionKind::RequiresApproval => State::RequiresApproval,
                    DecisionKind::NotFound | DecisionKind::Invalid => State::Rejected,
                    DecisionKind::Unknown(_) => State::Denied,
                };
                self.set(next)
            }
            State::RequiresApproval => {
                let has = d
                    .approval
                    .as_ref()
                    .is_some_and(|a| !a.approval_id.is_empty());
                if !has {
                    return Err(LifecycleError::MissingApproval);
                }
                match &d.decision {
                    DecisionKind::Authorized => self.set(State::Authorized),
                    DecisionKind::Denied | DecisionKind::Unknown(_) => self.set(State::Denied),
                    _ => Err(LifecycleError::Illegal { from: self.state }),
                }
            }
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
