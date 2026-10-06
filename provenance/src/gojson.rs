//! WALDO record identity, read from retained bytes.
//!
//! WALDO pins its records (`run_bom_sha256`, `bom_sha256`, `source_bom_sha256`, `plan_sha256`,
//! `corpus_bom_sha256`) as SHA-256 over Go `json.Marshal` of the record, while it writes the file
//! with `json.MarshalIndent` of the same value (openwaldo/waldo `internal/model/records.go`
//! `hashJSON` / `writeJSONAtomic`). Removing the insignificant whitespace of the file bytes gives
//! the `json.Marshal` bytes back, so the pin can be checked without re-encoding (and without
//! reordering keys). Checked against real WALDO run bytes: see provenance/README.md.

use sha2::{Digest, Sha256};

/// Lower-case hex SHA-256 of raw bytes.
pub fn sha256_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// Drop JSON whitespace outside strings. Bytes inside strings are kept exactly.
pub fn compact(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let (mut in_str, mut esc) = (false, false);
    for &c in b {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if !matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
            out.push(c);
            if c == b'"' {
                in_str = true;
            }
        }
    }
    out
}

/// WALDO record digest of a retained JSON file: SHA-256 of its compact form.
pub fn waldo_sha256(b: &[u8]) -> String {
    sha256_hex(&compact(b))
}

/// Index just past the JSON value that starts at `i` in compact bytes, or None if malformed.
fn value_end(c: &[u8], i: usize) -> Option<usize> {
    match *c.get(i)? {
        b'"' => {
            let mut j = i + 1;
            while j < c.len() {
                match c[j] {
                    b'\\' => j += 2,
                    b'"' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < c.len() {
                match c[j] {
                    b'"' => {
                        j = value_end(c, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < c.len() && !matches!(c[j], b',' | b'}' | b']') {
                j += 1;
            }
            Some(j)
        }
    }
}

/// The compact bytes of a top-level member's value, by exact key, or None.
pub fn top_level_member<'a>(compact_obj: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let c = compact_obj;
    if c.first() != Some(&b'{') {
        return None;
    }
    let want = format!("\"{key}\"");
    let mut i = 1;
    while i < c.len() && c[i] != b'}' {
        let key_end = value_end(c, i)?;
        if c.get(key_end) != Some(&b':') {
            return None;
        }
        let v_start = key_end + 1;
        let v_end = value_end(c, v_start)?;
        if &c[i..key_end] == want.as_bytes() {
            return Some(&c[v_start..v_end]);
        }
        i = v_end;
        if c.get(i) == Some(&b',') {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_keeps_string_bytes_and_order() {
        let b = b"{\n  \"b\": \"x y\\\" z\",\n  \"a\": [1, 2]\n}\n";
        assert_eq!(compact(b), b"{\"b\":\"x y\\\" z\",\"a\":[1,2]}".to_vec());
    }

    #[test]
    fn member_is_top_level_only() {
        let c = compact(b"{\"x\": {\"k\": 1}, \"k\": {\"z\": \"}\"}, \"k2\": 3}");
        assert_eq!(top_level_member(&c, "k"), Some(&b"{\"z\":\"}\"}"[..]));
        assert_eq!(top_level_member(&c, "k2"), Some(&b"3"[..]));
        assert_eq!(top_level_member(&c, "missing"), None);
    }
}
