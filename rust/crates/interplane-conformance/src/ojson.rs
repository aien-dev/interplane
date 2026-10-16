//! JSON that keeps object key order, and a writer with the byte form of Python's `json.dumps`.
//!
//! The corpus tools (fixture validator, subset rule, adapter runner) were first written in Python.
//! Two of their outputs depend on details `serde_json::Value` does not keep: the order of keys in an
//! object (the fixture validator checks it; the subset rule writes tool-call arguments in the
//! order of the translation table) and the exact text `json.dumps` produces (ASCII escapes,
//! `", "` and `": "` separators, Python float spelling). [`OJson`] keeps the order as read, and
//! [`dumps`] reproduces that text, so the frozen files those tools wrote stay byte-identical.
//!
//! Parsing is `serde_json` with `float_roundtrip`, so decimals parse to the nearest double as
//! Python's do. One known limit: an integer outside the 64-bit range parses as a float, where
//! Python kept it exact. No corpus file holds one. `serde_json`'s `arbitrary_precision` would lift
//! the limit but changes `serde_json::Number` for every crate in the workspace, so it is not used.
use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// A JSON value whose objects keep their keys in the order they were read or inserted.
#[derive(Debug, Clone, PartialEq)]
pub enum OJson {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Arr(Vec<OJson>),
    Obj(Vec<(String, OJson)>),
}

static NULL: OJson = OJson::Null;

impl OJson {
    pub fn parse(text: &str) -> Result<OJson, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn read(path: &std::path::Path) -> Result<OJson, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The value at `key`, or JSON null when this is not an object or has no such key.
    pub fn get(&self, key: &str) -> &OJson {
        self.get_opt(key).unwrap_or(&NULL)
    }

    pub fn get_opt(&self, key: &str) -> Option<&OJson> {
        match self {
            OJson::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut OJson> {
        match self {
            OJson::Obj(m) => m.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.get_opt(key).is_some()
    }

    /// Python `d[key] = value`: replace in place when present, else append.
    pub fn set(&mut self, key: &str, value: OJson) {
        if let OJson::Obj(m) = self {
            match m.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => m.push((key.to_string(), value)),
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            OJson::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> &[OJson] {
        match self {
            OJson::Arr(a) => a,
            _ => &[],
        }
    }

    pub fn entries(&self) -> &[(String, OJson)] {
        match self {
            OJson::Obj(m) => m,
            _ => &[],
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        self.entries().iter().map(|(k, _)| k.as_str()).collect()
    }

    pub fn is_obj(&self) -> bool {
        matches!(self, OJson::Obj(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, OJson::Null)
    }

    /// Python truthiness: null, false, 0, "", [] and {} are false.
    pub fn truthy(&self) -> bool {
        match self {
            OJson::Null => false,
            OJson::Bool(b) => *b,
            OJson::Int(i) => *i != 0,
            OJson::Float(f) => *f != 0.0,
            OJson::Str(s) => !s.is_empty(),
            OJson::Arr(a) => !a.is_empty(),
            OJson::Obj(m) => !m.is_empty(),
        }
    }

    /// The same value as a `serde_json::Value` (object key order is then sorted).
    pub fn to_value(&self) -> Value {
        match self {
            OJson::Null => Value::Null,
            OJson::Bool(b) => Value::Bool(*b),
            OJson::Int(i) => {
                if let Ok(u) = u64::try_from(*i) {
                    Value::Number(u.into())
                } else if let Ok(s) = i64::try_from(*i) {
                    Value::Number(s.into())
                } else {
                    Number::from_f64(*i as f64).map_or(Value::Null, Value::Number)
                }
            }
            OJson::Float(f) => Number::from_f64(*f).map_or(Value::Null, Value::Number),
            OJson::Str(s) => Value::String(s.clone()),
            OJson::Arr(a) => Value::Array(a.iter().map(OJson::to_value).collect()),
            OJson::Obj(m) => Value::Object(
                m.iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect::<Map<_, _>>(),
            ),
        }
    }

    /// From a `serde_json::Value` (object keys in that value's iteration order).
    pub fn from_value(v: &Value) -> OJson {
        match v {
            Value::Null => OJson::Null,
            Value::Bool(b) => OJson::Bool(*b),
            Value::Number(n) => match (n.as_i64(), n.as_u64()) {
                (Some(i), _) => OJson::Int(i as i128),
                (None, Some(u)) => OJson::Int(u as i128),
                _ => OJson::Float(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => OJson::Str(s.clone()),
            Value::Array(a) => OJson::Arr(a.iter().map(OJson::from_value).collect()),
            Value::Object(m) => OJson::Obj(
                m.iter()
                    .map(|(k, v)| (k.clone(), OJson::from_value(v)))
                    .collect(),
            ),
        }
    }
}

impl From<&str> for OJson {
    fn from(s: &str) -> Self {
        OJson::Str(s.to_string())
    }
}

impl From<String> for OJson {
    fn from(s: String) -> Self {
        OJson::Str(s)
    }
}

/// Build an [`OJson`] object from `(key, value)` pairs, in order.
pub fn obj<const N: usize>(pairs: [(&str, OJson); N]) -> OJson {
    OJson::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

impl<'de> Deserialize<'de> for OJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = OJson;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<OJson, E> {
                Ok(OJson::Null)
            }
            fn visit_none<E>(self) -> Result<OJson, E> {
                Ok(OJson::Null)
            }
            fn visit_bool<E>(self, b: bool) -> Result<OJson, E> {
                Ok(OJson::Bool(b))
            }
            fn visit_i64<E>(self, i: i64) -> Result<OJson, E> {
                Ok(OJson::Int(i as i128))
            }
            fn visit_u64<E>(self, u: u64) -> Result<OJson, E> {
                Ok(OJson::Int(u as i128))
            }
            fn visit_f64<E>(self, f: f64) -> Result<OJson, E> {
                Ok(OJson::Float(f))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<OJson, E> {
                Ok(OJson::Str(s.to_string()))
            }
            fn visit_string<E>(self, s: String) -> Result<OJson, E> {
                Ok(OJson::Str(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<OJson, A::Error> {
                let mut out = vec![];
                while let Some(v) = a.next_element()? {
                    out.push(v);
                }
                Ok(OJson::Arr(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<OJson, A::Error> {
                // Python's json.load: a repeated key keeps its first position and its last value.
                let mut out = OJson::Obj(vec![]);
                while let Some((k, v)) = a.next_entry::<String, OJson>()? {
                    out.set(&k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_any(V)
    }
}

/// How [`dumps`] lays the text out; each preset is one `json.dumps` call shape.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub sort_keys: bool,
    /// `separators=(",", ":")`.
    pub compact: bool,
    pub indent: Option<usize>,
}

impl Style {
    /// `json.dumps(x)`
    pub const PLAIN: Style = Style {
        sort_keys: false,
        compact: false,
        indent: None,
    };
    /// `json.dumps(x, sort_keys=True)`
    pub const SORTED: Style = Style {
        sort_keys: true,
        compact: false,
        indent: None,
    };
    /// `json.dumps(x, sort_keys=True, separators=(",", ":"))`
    pub const SORTED_COMPACT: Style = Style {
        sort_keys: true,
        compact: true,
        indent: None,
    };
    /// `json.dumps(x, indent=2, sort_keys=True)`
    pub const SORTED_INDENT2: Style = Style {
        sort_keys: true,
        compact: false,
        indent: Some(2),
    };
}

/// The text Python's `json.dumps` (default `ensure_ascii=True`) writes for `v` in `style`.
pub fn dumps(v: &OJson, style: Style) -> String {
    let mut out = String::new();
    write(v, style, 0, &mut out);
    out
}

fn write(v: &OJson, st: Style, level: usize, out: &mut String) {
    let (item_sep, key_sep) = match (st.compact, st.indent) {
        (true, _) => (",", ":"),
        (false, Some(_)) => (",", ": "),
        (false, None) => (", ", ": "),
    };
    let newline = |out: &mut String, lvl: usize| {
        if let Some(n) = st.indent {
            out.push('\n');
            out.push_str(&" ".repeat(n * lvl));
        }
    };
    match v {
        OJson::Null => out.push_str("null"),
        OJson::Bool(true) => out.push_str("true"),
        OJson::Bool(false) => out.push_str("false"),
        OJson::Int(i) => out.push_str(&i.to_string()),
        OJson::Float(f) => out.push_str(&py_float_repr(*f)),
        OJson::Str(s) => write_str(s, out),
        OJson::Arr(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline(out, level + 1);
                write(x, st, level + 1, out);
            }
            newline(out, level);
            out.push(']');
        }
        OJson::Obj(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut items: Vec<&(String, OJson)> = m.iter().collect();
            if st.sort_keys {
                items.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            for (i, (k, x)) in items.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline(out, level + 1);
                write_str(k, out);
                out.push_str(key_sep);
                write(x, st, level + 1, out);
            }
            newline(out, level);
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Python's `repr(float)`: the shortest round-trip digits, fixed notation for exponents -4 to 15,
/// scientific (`1e-05`, `1e+16`) outside it, and always a `.0` on whole numbers.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // Rust's `{:e}` prints the shortest round-trip digits, e.g. `-1.25e3`, `1e-7`.
    let sci = format!("{f:e}");
    let (mant, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("exponent");
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            let mut d = digits.clone();
            while d.len() < int_len {
                d.push('0');
            }
            s.push_str(&d[..int_len]);
            s.push('.');
            let frac = &d[int_len..];
            s.push_str(if frac.is_empty() { "0" } else { frac });
        } else {
            s.push_str("0.");
            s.push_str(&"0".repeat((-exp - 1) as usize));
            s.push_str(&digits);
        }
    } else {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if exp < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exp.abs()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_key_order_and_python_text() {
        let v = OJson::parse(r#"{"b": 1, "a": [1.5, "é😀", null], "b": 2}"#).unwrap();
        assert_eq!(v.keys(), ["b", "a"]);
        assert_eq!(
            dumps(&v, Style::PLAIN),
            r#"{"b": 2, "a": [1.5, "\u00e9\ud83d\ude00", null]}"#
        );
        assert_eq!(
            dumps(&v, Style::SORTED_COMPACT),
            r#"{"a":[1.5,"\u00e9\ud83d\ude00",null],"b":2}"#
        );
        assert_eq!(
            dumps(&v, Style::SORTED_INDENT2),
            "{\n  \"a\": [\n    1.5,\n    \"\\u00e9\\ud83d\\ude00\",\n    null\n  ],\n  \"b\": 2\n}"
        );
        assert_eq!(
            dumps(&OJson::parse("[{}, []]").unwrap(), Style::SORTED_INDENT2),
            "[\n  {},\n  []\n]"
        );
    }

    #[test]
    fn float_spelling_matches_python_repr() {
        for (f, want) in [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (123456789012345.6, "123456789012345.6"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (0.0, "0.0"),
            (1e22, "1e+22"),
        ] {
            assert_eq!(py_float_repr(f), want, "{f}");
        }
    }
}
