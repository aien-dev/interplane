//! RFC 8785 (JCS) canonical JSON and `sha256:` digests.
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};

/// Canonical JSON text: sorted keys (by UTF-16 code units), no whitespace, ES6 numbers.
pub fn canonicalize(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

/// `"sha256:" + lowercase hex` of the canonical JSON bytes.
pub fn digest(v: &Value) -> String {
    digest_bytes(canonicalize(v).as_bytes())
}

/// `"sha256:" + lowercase hex` of raw bytes.
pub fn digest_bytes(b: &[u8]) -> String {
    let h = Sha256::digest(b);
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for byte in h {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => write_string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(&m[k], out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    es6_f64(n.as_f64().unwrap_or(0.0))
}

/// ECMAScript `Number::toString` for finite doubles (NaN/Infinity cannot occur in JSON values).
pub fn es6_f64(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let neg = x < 0.0;
    let ax = x.abs();
    // Shortest round-trip digits via LowerExp: d.ddddde<exp>
    let e = format!("{ax:e}");
    let (mant, exp) = e.split_once('e').expect("lower exp");
    let exp: i32 = exp.parse().expect("exp");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if k <= n && n <= 21 {
        s.push_str(&digits);
        s.push_str(&"0".repeat((n - k) as usize));
    } else if 0 < n && n <= 21 {
        s.push_str(&digits[..n as usize]);
        s.push('.');
        s.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        s.push_str("0.");
        s.push_str(&"0".repeat((-n) as usize));
        s.push_str(&digits);
    } else {
        let sign = if n - 1 < 0 { '-' } else { '+' };
        s.push_str(&digits[..1]);
        if k > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(sign);
        s.push_str(&(n - 1).abs().to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_and_strips_whitespace() {
        assert_eq!(
            canonicalize(&json!({"b": 1, "a": [true, null, "x"]})),
            r#"{"a":[true,null,"x"],"b":1}"#
        );
    }
    #[test]
    fn numbers_es6() {
        assert_eq!(es6_f64(1e21), "1e+21");
        assert_eq!(es6_f64(1e20), "100000000000000000000");
        assert_eq!(es6_f64(1.5), "1.5");
        assert_eq!(es6_f64(0.000001), "0.000001");
        assert_eq!(es6_f64(0.0000001), "1e-7");
        assert_eq!(es6_f64(-2.5e-9), "-2.5e-9");
        assert_eq!(es6_f64(123456789012345680000.0), "123456789012345680000");
        assert_eq!(canonicalize(&json!(1.0)), "1.0".replace(".0", "")); // 1.0 parses as f64 -> "1"
        assert_eq!(canonicalize(&json!(-0.0)), "0");
        assert_eq!(canonicalize(&json!(10)), "10");
    }
    #[test]
    fn strings_escape() {
        assert_eq!(
            canonicalize(&json!("a\"b\\c\n\u{1}\u{7f}é😀")),
            "\"a\\\"b\\\\c\\n\\u0001\u{7f}é😀\""
        );
    }
    #[test]
    fn utf16_key_order() {
        // U+FFFD (BMP) sorts after U+1F600 (surrogate D83D) in UTF-16 order.
        let v = json!({"\u{fffd}": 1, "\u{1f600}": 2});
        assert_eq!(canonicalize(&v), "{\"\u{1f600}\":2,\"\u{fffd}\":1}");
    }
    #[test]
    fn digest_known() {
        assert_eq!(
            digest(&json!({})),
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }
}
