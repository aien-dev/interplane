//! Limits and the replay / duplicate ledger.
use crate::types::ErrorCode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Runtime-overridable limits (CORE.md). A model never raises them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default = "default_arg_bytes")]
    pub max_argument_bytes: usize,
    #[serde(default = "default_reqs")]
    pub max_requests_per_turn: usize,
}
fn default_arg_bytes() -> usize {
    65536
}
fn default_reqs() -> usize {
    32
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_argument_bytes: 65536,
            max_requests_per_turn: 32,
        }
    }
}

/// Tracks seen `message_id`s and `request_id`s per trace.
#[derive(Debug, Default, Clone)]
pub struct RequestLedger {
    seen: HashMap<String, (HashSet<String>, HashSet<String>)>,
}

impl RequestLedger {
    pub fn new() -> Self {
        Self::default()
    }
    /// Record a message id; a repeat within the trace is `replayed_message`.
    pub fn note_message(&mut self, trace: &str, message_id: &str) -> Result<(), ErrorCode> {
        let e = self.seen.entry(trace.to_string()).or_default();
        if e.0.insert(message_id.to_string()) {
            Ok(())
        } else {
            Err(ErrorCode::ReplayedMessage)
        }
    }
    /// Record a request id; a repeat within the trace is `duplicate_request_id`.
    pub fn note_request(&mut self, trace: &str, request_id: &str) -> Result<(), ErrorCode> {
        let e = self.seen.entry(trace.to_string()).or_default();
        if e.1.insert(request_id.to_string()) {
            Ok(())
        } else {
            Err(ErrorCode::DuplicateRequestId)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_and_duplicate() {
        let mut l = RequestLedger::new();
        assert!(l.note_message("t", "m1").is_ok());
        assert_eq!(l.note_message("t", "m1"), Err(ErrorCode::ReplayedMessage));
        assert!(l.note_message("u", "m1").is_ok(), "scoped per trace");
        assert!(l.note_request("t", "r1").is_ok());
        assert_eq!(
            l.note_request("t", "r1"),
            Err(ErrorCode::DuplicateRequestId)
        );
        assert!(
            l.note_request("t", "m1").is_ok(),
            "ids are separate namespaces"
        );
    }
    #[test]
    fn limits_defaults_and_partial_json() {
        assert_eq!(Limits::default().max_argument_bytes, 65536);
        assert_eq!(Limits::default().max_requests_per_turn, 32);
        let l: Limits = serde_json::from_str(r#"{"max_argument_bytes": 10}"#).unwrap();
        assert_eq!((l.max_argument_bytes, l.max_requests_per_turn), (10, 32));
    }
}
