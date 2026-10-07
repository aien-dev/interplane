//! The versioned byte forms the effect binding hashes. Producer (aien-cli, the daemon) and
//! verifier share these definitions; `provenance/BINDING.md` states them in prose.
//!
//! Nothing here is JCS. Each form below is named and fixed, and each function writes the exact
//! bytes the producer hashes. A form this module cannot reproduce byte for byte is refused
//! (`Err`), never approximated.
//!
//! Provenance is evidence, never authority: these functions compute digests and compare. They
//! decide and grant nothing.

use serde_json::Value;

/// Effect binding over the daemon-written ledger records (strong).
pub const LEDGER_BINDING: &str = "aien-ledger-slice/1";
/// Effect binding over aien-cli's loose `record_effect_receipt` v1 file (weaker).
pub const RECEIPT_BINDING: &str = "record_effect_receipt/1";
/// The daemon's approval binding version (sovereign-core `APPROVAL_BINDING_VERSION`).
pub const APPROVAL_BINDING: &str = "aien.approval.v2";
/// The digest form aien-cli's `record_effect_receipt` v1 uses: `serde_json::to_vec` on a map
/// whose keys iterate in sorted (UTF-8 byte) order.
pub const SERDE_DIGEST_FORM: &str = "serde_json.to_vec.sorted-keys/1";

/// RFC 8259 string with only the mandatory escapes: `"` and `\` get a backslash; U+0008, U+000C,
/// U+000A, U+000D, U+0009 use `\b \f \n \r \t`; every other control character below U+0020 is
/// `\u00xx` (lowercase hex). Everything else, including U+007F, U+2028 and all non-ASCII, is
/// written as UTF-8 unchanged. This is what `serde_json` writes.
pub fn json_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Form `serde_json.to_vec.sorted-keys/1`: compact JSON, object keys in UTF-8 byte order, no
/// whitespace, integers in decimal, strings as [`json_string`]. A number that is not an integer
/// is refused: the producer prints it with the shortest round-trip float algorithm, which this
/// verifier does not reimplement, so it cannot promise the same bytes.
pub fn compact_sorted(v: &Value) -> Result<Vec<u8>, String> {
    let mut o = String::new();
    write_sorted(v, &mut o)?;
    Ok(o.into_bytes())
}

fn write_sorted(v: &Value, o: &mut String) -> Result<(), String> {
    match v {
        Value::Null => o.push_str("null"),
        Value::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                o.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                o.push_str(&u.to_string());
            } else {
                return Err(format!("non-integer number {n}"));
            }
        }
        Value::String(s) => o.push_str(&json_string(s)),
        Value::Array(a) => {
            o.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                write_sorted(x, o)?;
            }
            o.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            o.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                o.push_str(&json_string(k));
                o.push(':');
                write_sorted(&m[*k], o)?;
            }
            o.push('}');
        }
    }
    Ok(())
}

/// The nine fields one approval covers, plus nothing else.
pub struct ApprovalFields<'a> {
    pub approval_id: &'a str,
    pub approved_proposal_sha256: &'a str,
    pub approver: &'a str,
    pub content_sha256: &'a str,
    pub desk_key_id: &'a str,
    pub path: &'a str,
    pub request_id: &'a str,
    pub trace_id: &'a str,
    pub workspace: &'a str,
}

/// The exact bytes the daemon hashes into the approval key (`aien.approval.v2`): one compact
/// JSON object, these ten keys in exactly this order (their UTF-8 byte order; `v` sorts before
/// `workspace`), every value a [`json_string`], `v` = "aien.approval.v2". The approval key is the SHA-256 of
/// these bytes.
pub fn approval_binding_bytes(f: &ApprovalFields<'_>) -> Vec<u8> {
    let mut o = String::from("{");
    for (k, v) in [
        ("approval_id", f.approval_id),
        ("approved_proposal_sha256", f.approved_proposal_sha256),
        ("approver", f.approver),
        ("content_sha256", f.content_sha256),
        ("desk_key_id", f.desk_key_id),
        ("path", f.path),
        ("request_id", f.request_id),
        ("trace_id", f.trace_id),
        ("v", APPROVAL_BINDING),
        ("workspace", f.workspace),
    ] {
        if o.len() > 1 {
            o.push(',');
        }
        o.push_str(&json_string(k));
        o.push(':');
        o.push_str(&json_string(v));
    }
    o.push('}');
    o.into_bytes()
}

/// `approved_proposal_sha256` input: the compact object `{"content":..,"path":..}` (those two
/// keys, in that order).
pub fn approved_proposal_bytes(path: &str, content: &str) -> Vec<u8> {
    format!(
        "{{\"content\":{},\"path\":{}}}",
        json_string(content),
        json_string(path)
    )
    .into_bytes()
}

/// `compose_proposal_sha256` input: the proposal text `filename: <path>\n<content>`.
pub fn compose_proposal_bytes(path: &str, content: &str) -> Vec<u8> {
    format!("filename: {path}\n{content}").into_bytes()
}
