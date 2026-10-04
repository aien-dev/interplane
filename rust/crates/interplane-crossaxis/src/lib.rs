//! CrossAxis: explicit capability mapping and the deterministic domain selector.
//! It never answers "may the model run it?".
use interplane_core::{
    canonicalize, CapabilityDescriptor, CapabilityRequest, CapabilityRequestKind, Catalog,
    ErrorCode, ExcludedEntry, MappingInfo, Measure, SelectedEntry, Selection, SelectionKind,
    SelectorInfo, ToolRef, ToolRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use thiserror::Error;

/// Rule kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Alias,
    Passthrough,
}

/// Canonical coordinates a rule matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleFrom {
    #[serde(default)]
    pub namespace: Option<String>,
    pub name: String,
}

/// An explicit argument rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

/// One mapping rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub kind: RuleKind,
    pub from: RuleFrom,
    pub to: String,
    #[serde(default)]
    pub renames: Vec<Rename>,
}

/// A versioned mapping table (data, not code).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingTable {
    pub runtime: String,
    #[serde(deserialize_with = "string_or_number")]
    pub table_version: String,
    /// Digest of the runtime catalog this table was built against (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_digest: Option<String>,
    pub rules: Vec<Rule>,
}

fn string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom(
            "table_version must be a string or number",
        )),
    }
}

/// Table loading failure.
#[derive(Debug, Error)]
pub enum MappingError {
    #[error("invalid mapping table: {0}")]
    Invalid(#[from] serde_json::Error),
    #[error("passthrough rule {0} must have a null namespace and `to` equal to `from.name`")]
    BadPassthrough(String),
}

impl MappingTable {
    pub fn from_json_str(s: &str) -> Result<Self, MappingError> {
        Self::from_value(serde_json::from_str(s)?)
    }
    pub fn from_value(v: Value) -> Result<Self, MappingError> {
        let t: MappingTable = serde_json::from_value(v)?;
        for r in &t.rules {
            if r.kind == RuleKind::Passthrough
                && (r.from.namespace.is_some() || r.from.name != r.to)
            {
                return Err(MappingError::BadPassthrough(r.id.clone()));
            }
        }
        Ok(t)
    }

    /// Exact lookup, first matching rule wins. No match -> `unknown_capability`.
    pub fn map(&self, req: &ToolRequest) -> Result<CapabilityRequest, ErrorCode> {
        let rule = self
            .rules
            .iter()
            .find(|r| r.from.namespace == req.tool.namespace && r.from.name == req.tool.name)
            .ok_or(ErrorCode::UnknownCapability)?;
        let mut arguments = req.arguments.clone();
        for rn in &rule.renames {
            if let Some(v) = arguments.remove(&rn.from) {
                arguments.insert(rn.to.clone(), v);
            }
        }
        Ok(CapabilityRequest {
            kind: CapabilityRequestKind,
            request_id: req.request_id.clone(),
            runtime: self.runtime.clone(),
            capability: rule.to.clone(),
            arguments,
            tool: ToolRef {
                namespace: req.tool.namespace.clone(),
                name: req.tool.name.clone(),
            },
            mapping: MappingInfo {
                table_version: self.table_version.clone(),
                rule_id: rule.id.clone(),
                passthrough: rule.kind == RuleKind::Passthrough,
                catalog_digest: self.catalog_digest.clone(),
                coerced: None,
                extensions: Map::new(),
            },
            runtime_effects: None,
            extensions: Map::new(),
        })
    }
}

fn schema_types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

/// Coerce string arguments to the integer/number/boolean the capability schema declares, when the
/// string parses. Records the coerced keys in `mapping.coerced` (sorted). Returns the keys.
pub fn coerce_arguments(req: &mut CapabilityRequest, cap: &CapabilityDescriptor) -> Vec<String> {
    let mut done = vec![];
    let Some(props) = cap.parameters.get("properties").and_then(Value::as_object) else {
        return done;
    };
    for (k, v) in req.arguments.iter_mut() {
        let (Some(schema), Value::String(s)) = (props.get(k), &*v) else {
            continue;
        };
        let types = schema_types(schema);
        if types.contains(&"string") {
            continue;
        }
        let new = if types.contains(&"integer") && s.parse::<i64>().is_ok() {
            Some(Value::from(s.parse::<i64>().unwrap_or_default()))
        } else if types.contains(&"number") {
            match (s.parse::<i64>(), s.parse::<f64>()) {
                (Ok(i), _) => Some(Value::from(i)),
                (_, Ok(f)) => Number::from_f64(f).map(Value::Number),
                _ => None,
            }
        } else if types.contains(&"boolean") && (s == "true" || s == "false") {
            Some(Value::Bool(s == "true"))
        } else {
            None
        };
        if let Some(n) = new {
            *v = n;
            done.push(k.clone());
        }
    }
    done.sort();
    if !done.is_empty() {
        req.mapping.coerced = Some(done.clone());
    }
    done
}

/// `domain_match` v1. Returns the receipt (without `measure`) and the selected descriptors.
pub fn select(
    catalog: &Catalog,
    requested_domains: &[String],
    max_capabilities: Option<usize>,
    always_include: &[String],
) -> (Selection, Vec<CapabilityDescriptor>) {
    let exclude = |name: &str, reason: &str| ExcludedEntry {
        name: name.into(),
        reason: reason.into(),
        extensions: Map::new(),
    };
    // 1. Pinned capabilities, in catalog order, never truncated.
    let mut selected: Vec<(&CapabilityDescriptor, String)> = catalog
        .capabilities
        .iter()
        .filter(|c| always_include.contains(&c.name))
        .map(|c| (c, "always_include".to_string()))
        .collect();
    let pinned = selected.len();
    // 2. Domain matches sorted by name.
    let mut domain: Vec<(&CapabilityDescriptor, String)> = vec![];
    let mut excluded: Vec<ExcludedEntry> = vec![];
    let mut rest: Vec<&CapabilityDescriptor> = catalog
        .capabilities
        .iter()
        .filter(|c| !always_include.contains(&c.name))
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    for c in rest {
        if c.domains.is_empty() {
            excluded.push(exclude(&c.name, "no_domains"));
        } else if let Some(d) = c.domains.iter().find(|d| requested_domains.contains(d)) {
            domain.push((c, format!("domain:{d}")));
        } else {
            excluded.push(exclude(&c.name, "domain_mismatch"));
        }
    }
    if let Some(max) = max_capabilities {
        let keep = max.saturating_sub(pinned).min(domain.len());
        for (c, _) in domain.drain(keep..) {
            excluded.push(exclude(&c.name, "max_capabilities"));
        }
    }
    selected.extend(domain);
    excluded.sort_by(|a, b| a.name.cmp(&b.name));
    let descriptors = selected.iter().map(|(c, _)| (*c).clone()).collect();
    let sel = Selection {
        kind: SelectionKind,
        runtime: catalog.runtime.clone(),
        catalog_digest: catalog
            .catalog_digest
            .clone()
            .unwrap_or_else(|| catalog.compute_digest()),
        selector: SelectorInfo {
            name: "domain_match".into(),
            version: "1".into(),
            max_capabilities: max_capabilities.map(|m| m as u64),
            extensions: Map::new(),
        },
        requested_domains: requested_domains.to_vec(),
        always_include: always_include.to_vec(),
        selected: selected
            .iter()
            .map(|(c, rule)| SelectedEntry {
                name: c.name.clone(),
                rule_id: rule.clone(),
                domains: c.domains.clone(),
                extensions: Map::new(),
            })
            .collect(),
        excluded,
        measure: None,
        extensions: Map::new(),
    };
    (sel, descriptors)
}

/// Fill `selection.measure`: render the full catalog and the selection with the same renderer and
/// count the canonical-JSON bytes of each rendering.
pub fn measure<F>(
    selection: &mut Selection,
    catalog: &Catalog,
    selected: &[CapabilityDescriptor],
    render: F,
) where
    F: Fn(&[CapabilityDescriptor]) -> Value,
{
    let bytes = |caps: &[CapabilityDescriptor]| canonicalize(&render(caps)).len() as u64;
    selection.measure = Some(Measure {
        full_count: Some(catalog.capabilities.len() as u64),
        selected_count: Some(selected.len() as u64),
        full_rendered_bytes: Some(bytes(&catalog.capabilities)),
        selected_rendered_bytes: Some(bytes(selected)),
        full_tokens: None,
        selected_tokens: None,
        tokenizer: None,
        extensions: Map::new(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table() -> MappingTable {
        MappingTable::from_value(json!({"runtime": "r", "table_version": "3", "rules": [
            {"id": "alias:filesystem.read", "kind": "alias", "from": {"namespace": "filesystem", "name": "read"}, "to": "read_file",
             "renames": [{"from": "file", "to": "path"}]},
            {"id": "passthrough:read_file", "kind": "passthrough", "from": {"namespace": null, "name": "read_file"}, "to": "read_file"}
        ]}))
        .unwrap()
    }
    fn req(ns: Option<&str>, name: &str, args: Value) -> ToolRequest {
        serde_json::from_value(
            json!({"kind":"tool_request","request_id":"r1","tool":{"namespace":ns,"name":name},
            "arguments":args,"provenance":{"dialect":"openai","parser_version":"1.0.0"}}),
        )
        .unwrap()
    }
    fn cap(name: &str, domains: &[&str], params: Value) -> CapabilityDescriptor {
        serde_json::from_value(
            json!({"name":name,"description":"d","parameters":params,"domains":domains}),
        )
        .unwrap()
    }
    fn catalog() -> Catalog {
        Catalog {
            kind: interplane_core::CatalogKind,
            runtime: "r".into(),
            catalog_version: "1".into(),
            catalog_digest: None,
            capabilities: vec![
                cap("write", &["filesystem"], json!({})),
                cap("send", &["email"], json!({})),
                cap("read", &["filesystem", "code"], json!({})),
                cap("bare", &[], json!({})),
            ],
            extensions: Map::new(),
        }
    }

    #[test]
    fn alias_passthrough_and_rename() {
        let m = table()
            .map(&req(Some("filesystem"), "read", json!({"file": "/a"})))
            .unwrap();
        assert_eq!(
            (
                m.capability.as_str(),
                m.mapping.rule_id.as_str(),
                m.mapping.passthrough
            ),
            ("read_file", "alias:filesystem.read", false)
        );
        assert_eq!(m.arguments["path"], json!("/a"));
        let p = table().map(&req(None, "read_file", json!({}))).unwrap();
        assert!(p.mapping.passthrough);
        assert_eq!(p.mapping.table_version, "3");
    }
    #[test]
    fn unknown_is_exact_not_fuzzy() {
        assert_eq!(
            table().map(&req(None, "read", json!({}))).unwrap_err(),
            ErrorCode::UnknownCapability
        );
        assert_eq!(
            table()
                .map(&req(Some("filesystem"), "READ", json!({})))
                .unwrap_err(),
            ErrorCode::UnknownCapability
        );
    }
    #[test]
    fn table_loading_negative_and_numeric_version() {
        assert!(MappingTable::from_json_str("{}").is_err());
        assert!(MappingTable::from_json_str("not json").is_err());
        let t =
            MappingTable::from_json_str(r#"{"runtime":"r","table_version":1,"rules":[]}"#).unwrap();
        assert_eq!(t.table_version, "1");
        let bad = json!({"runtime":"r","table_version":"1","rules":[{"id":"x","kind":"passthrough","from":{"name":"a"},"to":"b"}]});
        assert!(matches!(
            MappingTable::from_value(bad),
            Err(MappingError::BadPassthrough(_))
        ));
    }
    #[test]
    fn coercion_records_keys() {
        let c = cap(
            "f",
            &[],
            json!({"type":"object","properties":{"n":{"type":"integer"},"x":{"type":"number"},"b":{"type":"boolean"},"s":{"type":"string"},"bad":{"type":"integer"}}}),
        );
        let mut m = table()
            .map(&req(
                None,
                "read_file",
                json!({"n":"5","x":"2.5","b":"true","s":"7","bad":"abc","other":"1"}),
            ))
            .unwrap();
        let keys = coerce_arguments(&mut m, &c);
        assert_eq!(keys, vec!["b", "n", "x"]);
        assert_eq!(m.arguments["n"], json!(5));
        assert_eq!(m.arguments["x"], json!(2.5));
        assert_eq!(m.arguments["b"], json!(true));
        assert_eq!(m.arguments["s"], json!("7"));
        assert_eq!(m.arguments["bad"], json!("abc"));
        assert_eq!(
            m.mapping.coerced,
            Some(vec!["b".into(), "n".into(), "x".into()])
        );
    }
    #[test]
    fn select_domain_match_sorted_deterministic() {
        let (s, caps) = select(&catalog(), &["filesystem".to_string()], None, &[]);
        assert_eq!(
            caps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["read", "write"]
        );
        assert_eq!(s.selector.name, "domain_match");
        let reasons: Vec<(&str, &str)> = s
            .excluded
            .iter()
            .map(|e| (e.name.as_str(), e.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            [("bare", "no_domains"), ("send", "domain_mismatch")]
        );
        let (s2, _) = select(&catalog(), &["filesystem".to_string()], None, &[]);
        assert_eq!(s, s2);
    }
    #[test]
    fn select_max_and_always_include() {
        let (s, caps) = select(
            &catalog(),
            &["filesystem".to_string()],
            Some(2),
            &["send".to_string()],
        );
        assert_eq!(
            caps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["send", "read"],
            "pinned first, never truncated"
        );
        assert!(s
            .excluded
            .iter()
            .any(|e| e.name == "write" && e.reason == "max_capabilities"));
        assert_eq!(s.selected[0].rule_id, "always_include");
        assert_eq!(s.selected[1].rule_id, "domain:filesystem");
        let (_, caps) = select(&catalog(), &[], None, &["bare".to_string()]);
        assert_eq!(caps.len(), 1);
    }
    #[test]
    fn measure_counts_bytes() {
        let cat = catalog();
        let (mut s, caps) = select(&cat, &["email".to_string()], None, &[]);
        measure(&mut s, &cat, &caps, |c| {
            Value::Array(c.iter().map(|x| json!({"name": x.name})).collect())
        });
        let m = s.measure.unwrap();
        assert_eq!((m.full_count, m.selected_count), (Some(4), Some(1)));
        assert!(m.selected_rendered_bytes < m.full_rendered_bytes);
        assert_eq!(
            m.selected_rendered_bytes,
            Some(r#"[{"name":"send"}]"#.len() as u64)
        );
    }
}
