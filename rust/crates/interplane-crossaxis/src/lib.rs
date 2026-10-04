//! CrossAxis: explicit capability mapping and the deterministic domain selector.
//! It never answers "may the model run it?".
use interplane_core::{
    canonicalize, CapabilityDescriptor, CapabilityRequest, CapabilityRequestKind, Catalog,
    ErrorCode, ExcludedEntry, ExpansionBounds, ExpansionEntry, FullSelected, MappingInfo, Measure,
    RefusedEntry, SelectedEntry, Selection, SelectionKind, SelectorInfo, TokenMeasure, ToolRef,
    ToolRequest,
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
            expansion: None,
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
        expansions: vec![],
        parent_digest: None,
        selection_digest: None,
        extensions: Map::new(),
    };
    (sel, descriptors)
}

/// Render descriptors as OpenAI-style `tools` entries (the default measuring renderer; the same
/// shape as the Python `openai_tools_renderer`).
pub fn openai_tools_renderer(caps: &[CapabilityDescriptor]) -> Value {
    Value::Array(
        caps.iter()
            .map(|c| {
                serde_json::json!({"type": "function", "function": {
                    "name": c.name, "description": c.description, "parameters": c.parameters}})
            })
            .collect(),
    )
}

/// Bytes of `v` as sent on the wire: canonical JSON (RFC 8785), UTF-8.
fn wire_bytes(v: &Value) -> u64 {
    canonicalize(v).len() as u64
}

/// The unit values of `measure.tokens.unit`.
pub const TOKEN_UNITS: [&str; 4] = [
    "bytes",
    "tokens_model_reported",
    "tokens_endpoint_tokenizer",
    "tokens_estimated",
];
/// `source` recorded by [`EstimatedTokens`].
pub const ESTIMATOR_SOURCE: &str = "estimator v1 ceil(utf8_bytes/4)";

/// Counts one prompt. A live counter (model-reported `usage.prompt_tokens`, an endpoint
/// `/tokenize`) lives in the bench runner and implements this trait; Core stays offline.
/// `messages` is the JSON array of messages as sent, `tools` the rendered `tools` array (`None`
/// means no tools in the request).
pub trait TokenCounter {
    /// One of [`TOKEN_UNITS`].
    fn unit(&self) -> &str;
    /// Free text recorded in the receipt.
    fn source(&self) -> &str;
    fn count_prompt(&self, messages: &Value, tools: Option<&Value>) -> u64;
}

/// Unit `bytes`: canonical-JSON bytes of `messages` plus, when present, of `tools`.
pub struct BytesOnly;
impl TokenCounter for BytesOnly {
    fn unit(&self) -> &str {
        "bytes"
    }
    fn source(&self) -> &str {
        "canonical json bytes of messages and tools"
    }
    fn count_prompt(&self, messages: &Value, tools: Option<&Value>) -> u64 {
        wire_bytes(messages) + tools.map_or(0, wire_bytes)
    }
}

/// Unit `tokens_estimated`. Estimator v1: `ceil(utf8_len(jcs(value)) / 4)` per value; a prompt is
/// the estimate for `messages` plus, when present, the estimate for `tools`.
pub struct EstimatedTokens;
impl TokenCounter for EstimatedTokens {
    fn unit(&self) -> &str {
        "tokens_estimated"
    }
    fn source(&self) -> &str {
        ESTIMATOR_SOURCE
    }
    fn count_prompt(&self, messages: &Value, tools: Option<&Value>) -> u64 {
        let est = |v: &Value| wire_bytes(v).div_ceil(4);
        est(messages) + tools.map_or(0, est)
    }
}

/// Fill `selection.measure`: render the full catalog and the selection with the same renderer and
/// count the canonical-JSON bytes of each rendering (the bytes of `tools` as sent).
pub fn measure<F>(
    selection: &mut Selection,
    catalog: &Catalog,
    selected: &[CapabilityDescriptor],
    render: F,
) where
    F: Fn(&[CapabilityDescriptor]) -> Value,
{
    measure_tokens(selection, catalog, selected, render, None);
}

/// Totals the bench measured in the same unit as the counter, passed through into the receipt.
#[derive(Debug, Clone, Copy, Default)]
pub struct BenchTotals {
    pub all_rounds_prompt: Option<FullSelected>,
    pub completion: Option<FullSelected>,
}

/// [`measure`] plus `measure.tokens` when a counter and the base `messages` (system + user, as
/// sent) are given: `first_turn_prompt = count(messages, tools)`, `base_context = count(messages,
/// None)`, `tool_schema = first_turn_prompt - base_context`.
pub fn measure_tokens<F>(
    selection: &mut Selection,
    catalog: &Catalog,
    selected: &[CapabilityDescriptor],
    render: F,
    counted: Option<(&dyn TokenCounter, &Value, BenchTotals)>,
) where
    F: Fn(&[CapabilityDescriptor]) -> Value,
{
    let (full_r, sel_r) = (render(&catalog.capabilities), render(selected));
    let (f_bytes, s_bytes) = (wire_bytes(&full_r), wire_bytes(&sel_r));
    let tokens = counted.map(|(counter, messages, totals)| {
        let base = counter.count_prompt(messages, None);
        let first = FullSelected {
            full: counter.count_prompt(messages, Some(&full_r)),
            selected: counter.count_prompt(messages, Some(&sel_r)),
        };
        TokenMeasure {
            unit: counter.unit().to_string(),
            source: counter.source().to_string(),
            base_context: base,
            tool_schema: FullSelected {
                full: first.full.saturating_sub(base),
                selected: first.selected.saturating_sub(base),
            },
            first_turn_prompt: first,
            all_rounds_prompt: totals.all_rounds_prompt,
            completion: totals.completion,
        }
    });
    selection.measure = Some(Measure {
        full_count: Some(catalog.capabilities.len() as u64),
        selected_count: Some(selected.len() as u64),
        full_rendered_bytes: Some(f_bytes),
        selected_rendered_bytes: Some(s_bytes),
        full_tokens: None,
        selected_tokens: None,
        tokenizer: None,
        rendered_bytes: Some(FullSelected {
            full: f_bytes,
            selected: s_bytes,
        }),
        tokens,
        extensions: Map::new(),
    });
}

pub const DEFAULT_MAX_EXPANSIONS: u64 = 2;
pub const DEFAULT_MAX_ADDED_PER_EXPANSION: u64 = 8;
pub const DEFAULT_DISCOVER_LIMIT: usize = 8;

/// Evidence for [`expand`].
#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    /// The model called a capability that exists in the catalog but is not in the selection.
    RequestedExcluded {
        name: String,
        include_domain_siblings: bool,
    },
    /// Names returned by [`discover`] for `query`.
    DiscoveryHit {
        query: Option<String>,
        names: Vec<String>,
    },
}

impl Evidence {
    /// Parse the evidence JSON object (the form hashed into `evidence_digest`).
    pub fn from_value(v: &Value) -> Result<Self, ExpansionError> {
        let bad = |m: &str| ExpansionError(m.to_string());
        match v.get("kind").and_then(Value::as_str) {
            Some("requested_excluded") => Ok(Evidence::RequestedExcluded {
                name: v
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .ok_or_else(|| bad("requested_excluded needs a name"))?
                    .to_string(),
                include_domain_siblings: v.get("include_domain_siblings")
                    == Some(&Value::Bool(true)),
            }),
            Some("discovery_hit") => {
                let names = v
                    .get("names")
                    .and_then(Value::as_array)
                    .and_then(|a| {
                        a.iter()
                            .map(|n| n.as_str().map(str::to_string))
                            .collect::<Option<Vec<_>>>()
                    })
                    .ok_or_else(|| bad("discovery_hit needs names: [string]"))?;
                Ok(Evidence::DiscoveryHit {
                    query: v.get("query").and_then(Value::as_str).map(str::to_string),
                    names,
                })
            }
            other => Err(ExpansionError(format!("unknown evidence kind: {other:?}"))),
        }
    }
}

/// The evidence or selection is unusable (refusals are recorded in the receipt, not returned).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ExpansionError(pub String);

/// Digest of the selection with `selection_digest` and `measure` removed.
pub fn selection_digest(sel: &Selection) -> String {
    let mut v = serde_json::to_value(sel).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("selection_digest");
        o.remove("measure");
    }
    interplane_core::digest(&v)
}

/// Names of EXCLUDED capabilities whose name or description contains `query` (ASCII
/// case-insensitive substring), sorted by name, at most `limit`. `runtime_disabled` exclusions are
/// never offered; an empty query gives `[]`. Pure.
pub fn discover(
    catalog: &Catalog,
    selection: &Selection,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut hits: Vec<String> = catalog
        .capabilities
        .iter()
        .filter(|c| {
            selection
                .excluded
                .iter()
                .any(|e| e.name == c.name && e.reason != "runtime_disabled")
        })
        .filter(|c| {
            c.name.to_ascii_lowercase().contains(&q)
                || c.description.to_ascii_lowercase().contains(&q)
        })
        .map(|c| c.name.clone())
        .collect();
    hits.sort();
    hits.truncate(limit);
    hits
}

/// Bounded evidence-based expansion; returns a new selection (the input is untouched). Bounds
/// recorded in `selector.expansion` win over `max_expansions` / `max_added_per_expansion`, which
/// win over the defaults (2 and 8); the effective bounds are recorded. `evidence_json` is the
/// evidence object whose digest goes into the receipt. Refusal reasons and order: spec/CROSSAXIS.md.
pub fn expand(
    selection: &Selection,
    catalog: &Catalog,
    evidence_json: &Value,
    max_expansions: Option<u64>,
    max_added_per_expansion: Option<u64>,
) -> Result<Selection, ExpansionError> {
    let evidence = Evidence::from_value(evidence_json)?;
    let cat_digest = catalog
        .catalog_digest
        .clone()
        .unwrap_or_else(|| catalog.compute_digest());
    if selection.catalog_digest != cat_digest {
        return Err(ExpansionError(
            "selection was made against a different catalog".into(),
        ));
    }
    let find = |n: &str| catalog.capabilities.iter().find(|c| c.name == n);
    let (kind, candidates): (&str, Vec<String>) = match &evidence {
        Evidence::RequestedExcluded {
            name,
            include_domain_siblings,
        } => {
            let mut c = vec![name.clone()];
            if *include_domain_siblings {
                if let Some(cap) = find(name) {
                    let mut sib: Vec<String> = catalog
                        .capabilities
                        .iter()
                        .filter(|o| {
                            o.name != *name && o.domains.iter().any(|d| cap.domains.contains(d))
                        })
                        .map(|o| o.name.clone())
                        .collect();
                    sib.sort();
                    c.extend(sib);
                }
            }
            ("requested_excluded", c)
        }
        Evidence::DiscoveryHit { names, .. } => ("discovery_hit", names.clone()),
    };

    let mut out = selection.clone();
    let parent = selection_digest(selection);
    let rec = out.selector.expansion.clone();
    let max_exp = rec
        .as_ref()
        .map(|b| b.max_expansions)
        .or(max_expansions)
        .unwrap_or(DEFAULT_MAX_EXPANSIONS);
    let max_add = rec
        .as_ref()
        .map(|b| b.max_added_per_expansion)
        .or(max_added_per_expansion)
        .unwrap_or(DEFAULT_MAX_ADDED_PER_EXPANSION);
    out.selector.expansion = Some(ExpansionBounds {
        max_expansions: max_exp,
        max_added_per_expansion: max_add,
    });

    let used = out
        .expansions
        .iter()
        .filter(|e| !e.added.is_empty())
        .count() as u64;
    let round = out.expansions.len() as u64 + 1;
    let cap_limit = out.selector.max_capabilities;
    let mut added: Vec<String> = vec![];
    let mut refused: Vec<RefusedEntry> = vec![];
    let mut seen: Vec<&String> = vec![];
    for name in &candidates {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        let reason = if find(name).is_none() {
            Some("not_in_catalog")
        } else if out.selected.iter().any(|e| &e.name == name) {
            Some("already_selected")
        } else if out
            .excluded
            .iter()
            .any(|e| &e.name == name && e.reason == "runtime_disabled")
        {
            Some("runtime_disabled")
        } else if used >= max_exp {
            Some("max_expansions")
        } else if added.len() as u64 >= max_add {
            Some("max_added_per_expansion")
        } else if cap_limit.is_some_and(|m| (out.selected.len() + added.len()) as u64 >= m) {
            Some("max_capabilities")
        } else {
            None
        };
        match reason {
            None => added.push(name.clone()),
            Some(r) => refused.push(RefusedEntry {
                name: name.clone(),
                reason: r.into(),
            }),
        }
    }
    for name in &added {
        if let Some(c) = find(name) {
            out.selected.push(SelectedEntry {
                name: c.name.clone(),
                rule_id: format!("expansion:{round}"),
                domains: c.domains.clone(),
                extensions: Map::new(),
            });
        }
    }
    out.excluded.retain(|e| !added.contains(&e.name));
    out.expansions.push(ExpansionEntry {
        round,
        reason: kind.into(),
        evidence_digest: interplane_core::digest(evidence_json),
        added,
        refused,
    });
    out.measure = None;
    out.parent_digest = Some(parent);
    out.selection_digest = None;
    out.selection_digest = Some(selection_digest(&out));
    Ok(out)
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

    // ---- measurement
    fn msgs() -> Value {
        json!([{"role":"system","content":"s"},{"role":"user","content":"find the readme"}])
    }
    #[test]
    fn measure_bytes_are_wire_bytes_and_tokens_are_absent_without_a_counter() {
        let cat = catalog();
        let (mut s, caps) = select(&cat, &["email".to_string()], None, &[]);
        measure(&mut s, &cat, &caps, openai_tools_renderer);
        let m = s.measure.clone().unwrap();
        assert_eq!(
            m.rendered_bytes.unwrap().full,
            canonicalize(&openai_tools_renderer(&cat.capabilities)).len() as u64
        );
        assert_eq!(m.full_rendered_bytes, Some(m.rendered_bytes.unwrap().full));
        assert!(m.tokens.is_none());
        let v = serde_json::to_value(&s).unwrap();
        assert!(v["measure"].get("tokens").is_none() && v["measure"].get("full_tokens").is_none());
        assert!(v.get("always_include").is_none());
        assert_eq!(
            v["selector"],
            json!({"name": "domain_match", "version": "1"})
        );
    }
    #[test]
    fn estimated_tokens_exact_and_differential() {
        let cat = catalog();
        let (mut s, caps) = select(&cat, &["email".to_string()], None, &[]);
        let m = msgs();
        measure_tokens(
            &mut s,
            &cat,
            &caps,
            openai_tools_renderer,
            Some((&EstimatedTokens, &m, BenchTotals::default())),
        );
        let t = s.measure.unwrap().tokens.unwrap();
        let est = |v: &Value| (canonicalize(v).len() as u64).div_ceil(4);
        assert_eq!(
            (t.unit.as_str(), t.source.as_str()),
            ("tokens_estimated", ESTIMATOR_SOURCE)
        );
        assert_eq!(t.base_context, est(&m));
        let (full_t, sel_t) = (
            est(&openai_tools_renderer(&cat.capabilities)),
            est(&openai_tools_renderer(&caps)),
        );
        assert_eq!(
            (t.tool_schema.full, t.tool_schema.selected),
            (full_t, sel_t)
        );
        assert_eq!(t.first_turn_prompt.full, t.base_context + full_t);
        assert!(TOKEN_UNITS.contains(&"bytes") && t.all_rounds_prompt.is_none());
        // the Python implementation computes the same numbers from the same inputs
        assert_eq!(EstimatedTokens.count_prompt(&json!([]), None), 1);
        assert_eq!(BytesOnly.count_prompt(&json!([]), Some(&json!({}))), 4);
    }

    // ---- expansion
    fn email_sel(max: Option<usize>) -> (Catalog, Selection) {
        let cat = catalog();
        let (s, _) = select(&cat, &["email".to_string()], max, &[]);
        (cat, s)
    }
    fn big() -> Catalog {
        let mut c = catalog();
        c.capabilities.push(cap("list", &["filesystem"], json!({})));
        c.capabilities
            .push(cap("trash", &["filesystem"], json!({})));
        c
    }
    fn rx(name: &str) -> Value {
        json!({"kind": "requested_excluded", "name": name})
    }
    #[test]
    fn expand_adds_records_and_chains_digests() {
        let (cat, sel) = email_sel(None);
        let before = sel.clone();
        let out = expand(&sel, &cat, &rx("write"), None, None).unwrap();
        assert_eq!(sel, before, "input untouched");
        let names: Vec<_> = out.selected.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["send", "write"]);
        assert_eq!(out.selected[1].rule_id, "expansion:1");
        assert!(!out.excluded.iter().any(|e| e.name == "write"));
        let e = &out.expansions[0];
        assert_eq!(
            (e.round, e.reason.as_str(), e.added.clone()),
            (1, "requested_excluded", vec!["write".to_string()])
        );
        assert_eq!(e.evidence_digest, interplane_core::digest(&rx("write")));
        assert_eq!(out.parent_digest, Some(selection_digest(&sel)));
        assert_eq!(out.selection_digest, Some(selection_digest(&out)));
        assert_eq!(
            out.selector.expansion,
            Some(ExpansionBounds {
                max_expansions: 2,
                max_added_per_expansion: 8
            })
        );
        let again = expand(&sel, &cat, &rx("write"), None, None).unwrap();
        assert_eq!(
            canonicalize(&serde_json::to_value(&again).unwrap()),
            canonicalize(&serde_json::to_value(&out).unwrap())
        );
        let out2 = expand(&out, &cat, &rx("read"), None, None).unwrap();
        assert_eq!(out2.parent_digest, out.selection_digest);
        assert_eq!(
            out2.expansions.iter().map(|e| e.round).collect::<Vec<_>>(),
            [1, 2]
        );
    }
    #[test]
    fn expand_drops_measure_and_rejects_a_foreign_catalog_and_bad_evidence() {
        let cat = catalog();
        let (mut s, caps) = select(&cat, &["email".to_string()], None, &[]);
        measure(&mut s, &cat, &caps, openai_tools_renderer);
        assert!(expand(&s, &cat, &rx("read"), None, None)
            .unwrap()
            .measure
            .is_none());
        let mut other = catalog();
        other.capabilities.pop();
        other.catalog_digest = None;
        assert!(expand(&s, &other, &rx("read"), None, None).is_err());
        for bad in [
            json!({}),
            json!({"kind": "x"}),
            json!({"kind": "requested_excluded"}),
            json!({"kind": "discovery_hit"}),
        ] {
            assert!(expand(&s, &cat, &bad, None, None).is_err());
        }
    }
    #[test]
    fn expand_refusal_reasons_and_order() {
        let cat = catalog();
        let (_, sel) = email_sel(Some(2));
        let ev = json!({"kind": "discovery_hit", "names": ["nope", "send", "read", "write"]});
        let last = expand(&sel, &cat, &ev, None, None)
            .unwrap()
            .expansions
            .pop()
            .unwrap();
        assert_eq!(last.added, ["read"]);
        let r: Vec<_> = last
            .refused
            .iter()
            .map(|r| (r.name.as_str(), r.reason.as_str()))
            .collect();
        assert_eq!(
            r,
            [
                ("nope", "not_in_catalog"),
                ("send", "already_selected"),
                ("write", "max_capabilities")
            ]
        );
        // max_added_per_expansion
        let cat = big();
        let (s2, _) = select(&cat, &["email".to_string()], None, &[]);
        let ev = json!({"kind": "discovery_hit", "names": ["read", "list", "trash"]});
        let last = expand(&s2, &cat, &ev, None, Some(2))
            .unwrap()
            .expansions
            .pop()
            .unwrap();
        assert_eq!(last.added, ["read", "list"]);
        assert_eq!(last.refused[0].reason, "max_added_per_expansion");
        // max_expansions: refused rounds are recorded, count in numbering, and add nothing
        let mut s = s2;
        for n in ["read", "list"] {
            s = expand(&s, &cat, &rx(n), None, None).unwrap();
        }
        let s3 = expand(&s, &cat, &rx("trash"), Some(99), None).unwrap();
        let last = s3.expansions.last().unwrap();
        assert_eq!((last.round, last.added.len()), (3, 0));
        assert_eq!(last.refused[0].reason, "max_expansions");
        assert!(
            !s3.selected.iter().any(|e| e.name == "trash"),
            "recorded bounds win over arguments"
        );
    }
    #[test]
    fn expand_siblings_runtime_disabled_and_discover() {
        let (cat, sel) = email_sel(None);
        let ev =
            json!({"kind": "requested_excluded", "name": "write", "include_domain_siblings": true});
        let last = expand(&sel, &cat, &ev, None, None)
            .unwrap()
            .expansions
            .pop()
            .unwrap();
        assert_eq!(last.added, ["write", "read"]);
        let mut dis = sel.clone();
        dis.excluded
            .iter_mut()
            .find(|e| e.name == "read")
            .unwrap()
            .reason = "runtime_disabled".into();
        assert!(!discover(&cat, &dis, "d", 8).contains(&"read".to_string()));
        let last = expand(&dis, &cat, &rx("read"), None, None)
            .unwrap()
            .expansions
            .pop()
            .unwrap();
        assert_eq!(last.refused[0].reason, "runtime_disabled");
        // discover: ASCII case-insensitive over name and description, sorted, capped, excluded only
        let cat = big();
        let (s, _) = select(&cat, &["email".to_string()], None, &[]);
        assert_eq!(
            discover(&cat, &s, "R", 8),
            ["bare", "read", "trash", "write"]
        );
        assert_eq!(
            discover(&cat, &s, "D", 8),
            ["bare", "list", "read", "trash", "write"]
        );
        assert_eq!(discover(&cat, &s, "d", 2), ["bare", "list"]);
        assert!(discover(&cat, &s, "send", 8).is_empty() && discover(&cat, &s, "", 8).is_empty());
    }
}
