//! Strict JSON for retained bytes: exactly one value, and no object repeats a key at any depth.
//! A last-wins reader (serde_json's `Value`) and a first-wins reader could see different values
//! in the same bytes, so a duplicate key makes the record unparseable here (#76 review).

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrictVisitor).map(Strict)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value without duplicate object keys")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }
    fn visit_i64<E>(self, n: i64) -> Result<Value, E> {
        Ok(n.into())
    }
    fn visit_u64<E>(self, n: u64) -> Result<Value, E> {
        Ok(n.into())
    }
    fn visit_f64<E: de::Error>(self, n: f64) -> Result<Value, E> {
        Number::from_f64(n)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::String(s.to_string()))
    }
    fn visit_string<E>(self, s: String) -> Result<Value, E> {
        Ok(Value::String(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(Strict(v)) = a.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = Map::new();
        while let Some(k) = a.next_key::<String>()? {
            let Strict(v) = a.next_value()?;
            if out.contains_key(&k) {
                return Err(de::Error::custom(format!("duplicate key {k:?}")));
            }
            out.insert(k, v);
        }
        Ok(Value::Object(out))
    }
}

/// Parse `b` as one JSON value; None on any syntax error, trailing data or duplicate key.
pub fn parse(b: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Strict>(b).ok().map(|s| s.0)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn duplicate_keys_are_refused_at_any_depth() {
        assert!(parse(br#"{"a":1,"b":[{"c":true}]}"#).is_some());
        assert!(parse(br#"{"a":1,"a":2}"#).is_none());
        assert!(parse(br#"{"x":{"a":1,"a":1}}"#).is_none());
        assert!(parse(br#"[{"a":1},{"b":{"c":[{"d":1,"d":2}]}}]"#).is_none());
        // An escaped spelling of the same key is the same key.
        assert!(parse(br#"{"a":1,"a":2}"#).is_none());
        assert!(parse(br#"{"a":1} {}"#).is_none());
        assert_eq!(
            parse(br#"{"n":-3,"u":18446744073709551615,"f":0.5,"z":null}"#).unwrap(),
            serde_json::json!({"n":-3,"u":18446744073709551615u64,"f":0.5,"z":null})
        );
    }
}
