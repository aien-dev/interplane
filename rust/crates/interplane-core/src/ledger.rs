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
    /// Session state limits (CORE.md): traces held at once.
    #[serde(default = "default_traces")]
    pub max_traces: usize,
    #[serde(default = "default_session")]
    pub max_messages_per_trace: usize,
    #[serde(default = "default_session")]
    pub max_requests_per_trace: usize,
    #[serde(default = "default_session")]
    pub max_inputs_per_trace: usize,
    /// Closed trace ids remembered exactly (CORE.md, "Closing a trace"); older ones are retired.
    #[serde(default = "default_session")]
    pub max_closed_traces: usize,
    /// Size in bits of the filter that keeps retired closed trace ids refused.
    #[serde(default = "default_filter_bits")]
    pub retired_filter_bits: usize,
}
fn default_filter_bits() -> usize {
    8_388_608
}
fn default_traces() -> usize {
    1024
}
fn default_session() -> usize {
    4096
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
            max_traces: 1024,
            max_messages_per_trace: 4096,
            max_requests_per_trace: 4096,
            max_inputs_per_trace: 4096,
            max_closed_traces: 4096,
            retired_filter_bits: 8_388_608,
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
    /// Read-only views and cleanup used by the session state limits (CORE.md).
    pub fn has_message(&self, trace: &str, message_id: &str) -> bool {
        self.seen
            .get(trace)
            .is_some_and(|e| e.0.contains(message_id))
    }
    pub fn has_request(&self, trace: &str, request_id: &str) -> bool {
        self.seen
            .get(trace)
            .is_some_and(|e| e.1.contains(request_id))
    }
    pub fn message_count(&self, trace: &str) -> usize {
        self.seen.get(trace).map_or(0, |e| e.0.len())
    }
    pub fn request_count(&self, trace: &str) -> usize {
        self.seen.get(trace).map_or(0, |e| e.1.len())
    }
    /// True when the trace holds at least one recorded id.
    pub fn holds(&self, trace: &str) -> bool {
        self.message_count(trace) + self.request_count(trace) > 0
    }
    /// Every trace that holds at least one recorded id.
    pub fn traces(&self) -> impl Iterator<Item = &String> {
        self.seen
            .iter()
            .filter(|(_, e)| !e.0.is_empty() || !e.1.is_empty())
            .map(|(t, _)| t)
    }
    pub fn forget(&mut self, trace: &str) {
        self.seen.remove(trace);
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
