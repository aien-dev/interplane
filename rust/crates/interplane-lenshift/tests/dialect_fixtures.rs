use interplane_lenshift::{DialectRegistry, ParseContext};
use serde_json::{json, Value};
use std::path::PathBuf;

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("source_digest");
            m.retain(|_, x| !x.is_null());
            m.values_mut().for_each(strip);
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        _ => {}
    }
}

#[test]
fn dialect_fixtures_match() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dialects/fixtures");
    let reg = DialectRegistry::with_defaults();
    let mut n = 0;
    for dir in std::fs::read_dir(&root).expect("dialects/fixtures present") {
        let dir = dir.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap() {
            let path = f.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let fx: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let dialect = fx["dialect"].as_str().unwrap();
            let ctx = ParseContext {
                trace_id: fx["trace_id"].as_str().unwrap_or("trace-dialect").into(),
                turn: fx["turn"].as_u64().unwrap_or(0),
                model: fx["model"].as_str().unwrap_or("").into(),
            };
            let t = reg.get(dialect).unwrap().parse(&fx["input"], &ctx);
            let name = path.display().to_string();
            let e = &fx["expected"];
            assert_eq!(json!(t.text), e["text"], "{name} text");
            assert_eq!(json!(t.partial), e["partial"], "{name} partial");
            assert_eq!(
                json!(t.dialect_version),
                e["dialect_version"],
                "{name} dialect_version"
            );
            assert_eq!(
                json!(t.parser_version),
                fx["parser_version"],
                "{name} parser_version"
            );
            assert_eq!(
                t.reasoning_digest.is_some(),
                fx["reasoning_present"].as_bool().unwrap_or(false),
                "{name} reasoning"
            );
            let mut got = serde_json::to_value(&t.intents).unwrap();
            let mut want = e["intents"].clone();
            strip(&mut got);
            strip(&mut want);
            assert_eq!(got, want, "{name} intents");
            let rej: Vec<Value> = t
                .rejected
                .iter()
                .map(|r| json!({"index": r.index, "code": r.code, "message": r.message}))
                .collect();
            let mut want_rej = e["rejected"].clone();
            if let Some(a) = want_rej.as_array_mut() {
                for x in a {
                    x.as_object_mut().unwrap().remove("source_digest");
                }
            }
            assert_eq!(Value::Array(rej), want_rej, "{name} rejected");
            n += 1;
        }
    }
    assert!(n >= 18, "expected the 18 dialect fixtures, ran {n}");
}
