//! INTERPLANE 0.1 Core: wire types, canonical JSON, validation, lifecycle.
//!
//! Core executes nothing and authorizes nothing. A valid envelope means
//! "structurally understandable", never "authorized".

#[macro_use]
mod macros;
pub mod jcs;
pub mod ledger;
pub mod lifecycle;
pub mod types;
pub mod validate;
pub mod version;

pub use jcs::{canonicalize, digest, digest_bytes};
pub use ledger::{Limits, RequestLedger};
pub use lifecycle::{Lifecycle, LifecycleError, State};
pub use types::*;
pub use validate::{is_valid_id, validate_envelope, validate_envelope_value};
pub use version::{ProtocolVersion, PROTOCOL_VERSION, SUPPORTED_MAJOR};
