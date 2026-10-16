//! The T3 and T4 subset rule and fixture translation (bench/PROTOCOL-0.3.md section 2; the normative
//! text is spec/CORE.md, "Adapter subsets").
//!
//! The rule reads only fixture metadata and `conformance/adapter-translation.json`, never a
//! result. The AIEN T4 runner and [`crate::adapter_runner`] execute exactly these plans. Values
//! keep their key order ([`OJson`]) because translated tool-call arguments are written back as
//! text in the order of the table, and `interplane-corpus subset --plans` must match the frozen
//! byte form.
use std::path::{Path, PathBuf};

use regex::{Captures, Regex};
use sha2::{Digest, Sha256};

use crate::ojson::{dumps, obj, OJson, Style};

fn qwen() -> Regex {
    Regex::new(r"(?s)<tool_call>\s*<function=(\w+)>(.*?)</function>\s*</tool_call>").unwrap()
}

fn param() -> Regex {
    Regex::new(r"(?s)<parameter=(\w+)>\n?(.*?)\n?</parameter>").unwrap()
}

/// The repository paths the rule reads.
#[derive(Debug, Clone)]
pub struct Corpus {
    pub root: PathBuf,
}

impl Corpus {
    pub fn new(root: &Path) -> Self {
        Corpus {
            root: root.to_path_buf(),
        }
    }

    pub fn load_table(&self) -> Result<OJson, String> {
        OJson::read(&self.root.join("conformance/adapter-translation.json"))
    }

    /// `(name, case)` for the injection and approval fixtures, `injection/NN-slug` order then approval.
    pub fn fixtures(&self) -> Result<Vec<(String, OJson)>, String> {
        let mut out = vec![];
        for sub in ["injection", "approval"] {
            let dir = self.root.join("conformance/fixtures").join(sub);
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".json"))
                .collect();
            names.sort();
            for n in names {
                let case = OJson::read(&dir.join(&n))?;
                out.push((format!("{sub}/{}", &n[..n.len() - 5]), case));
            }
        }
        Ok(out)
    }

    /// The fixtures `adapter` must run, in corpus order.
    pub fn subset(&self, adapter: &str, table: &OJson) -> Result<Vec<String>, String> {
        let mut out = vec![];
        for (n, c) in self.fixtures()? {
            if decide_subset(&n, &c, adapter, table)?.0 {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// The translated plans of `adapter`'s subset.
    pub fn plans(&self, adapter: &str, table: &OJson) -> Result<Vec<OJson>, String> {
        let fixtures = self.fixtures()?;
        let mut out = vec![];
        for (n, c) in &fixtures {
            if decide_subset(n, c, adapter, table)?.0 {
                out.push(translate(n, c, adapter, table)?);
            }
        }
        Ok(out)
    }
}

fn strs(v: &OJson) -> Vec<&str> {
    v.as_arr().iter().filter_map(OJson::as_str).collect()
}

fn adapter_entry<'a>(table: &'a OJson, adapter: &str) -> Result<&'a OJson, String> {
    table
        .get("adapters")
        .get_opt(adapter)
        .ok_or_else(|| format!("{adapter} is not in the translation table"))
}

/// `[(capability, shape)]` a step calls, or `None` when the step is not an openai, qwen35 or
/// envelope call.
fn calls(step: &OJson) -> Option<Vec<(Option<String>, &'static str)>> {
    if step.has("envelope") {
        let name = step.get("envelope").get("payload").get("tool").get("name");
        return Some(vec![(name.as_str().map(String::from), "envelope")]);
    }
    if step.has("dialect") {
        return match step.get("dialect").as_str() {
            Some("openai") => Some(
                step.get("input")
                    .get("tool_calls")
                    .as_arr()
                    .iter()
                    .map(|c| {
                        (
                            c.get("function").get("name").as_str().map(String::from),
                            "openai",
                        )
                    })
                    .collect(),
            ),
            Some("qwen35") => Some(
                qwen()
                    .captures_iter(step.get("input").as_str().unwrap_or(""))
                    .map(|m| (Some(m[1].to_string()), "qwen35"))
                    .collect(),
            ),
            _ => None,
        };
    }
    Some(vec![])
}

/// `(in_subset, reason)`. The rule, in order; the first failing clause names the reason.
pub fn decide_subset(
    name: &str,
    case: &OJson,
    adapter: &str,
    table: &OJson,
) -> Result<(bool, Option<String>), String> {
    let ad = adapter_entry(table, adapter)?;
    let is_approval = name.starts_with("approval/");
    let is_pf = strs(table.get("provider_failure_cases")).contains(&name);
    let no = |r: String| Ok((false, Some(r)));
    if is_pf && !ad.has("provider_failure_effects") {
        return no("provider-failure case (protocol A15) runs on T4 only".into());
    }
    if strs(table.get("harness_keys"))
        .iter()
        .filter(|k| !(is_pf && **k == "mock_fault"))
        .any(|k| case.has(k))
    {
        return no("carries a harness-only mock key".into());
    }
    let harness_steps = strs(table.get("harness_steps"));
    for s in case.get("steps").as_arr() {
        if harness_steps.iter().any(|k| s.has(k)) {
            return no("has a harness-only mock step".into());
        }
    }
    let classes = if is_approval {
        let num: i64 = name
            .split('/')
            .nth(1)
            .and_then(|s| s.get(..2))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("{name}: approval fixture without a number"))?;
        let range = table.get("approval_cases");
        let (first, last) = (int(range.get("first")), int(range.get("last")));
        if !is_pf && !(first <= num && num <= last) {
            return no("not a protocol approval case (A01 to A14)".into());
        }
        ad.get(if is_pf {
            "provider_failure_effects"
        } else {
            "approval_effects"
        })
    } else {
        let target = case
            .get("injection")
            .get("target")
            .get("capability")
            .as_str()
            .unwrap_or("");
        if !ad.get("effects").has(target) {
            return no(format!(
                "target capability class {target} not in the adapter catalog"
            ));
        }
        ad.get("effects")
    };
    let mock_only = strs(table.get("mock_only_capabilities"));
    for s in case.get("steps").as_arr() {
        let Some(cs) = calls(s) else {
            return no("dialect step the rule cannot translate".into());
        };
        for (cap, shape) in cs {
            let cap = cap.unwrap_or_default();
            if mock_only.contains(&cap.as_str()) {
                return no(format!("uses mock-only capability {cap}"));
            }
            if classes.has(&cap) || ad.get("native_reads").has(&cap) {
                continue;
            }
            if table.get("delivered_reads").has(&cap) && shape != "envelope" {
                continue;
            }
            return no(format!("capability {cap} has no counterpart"));
        }
    }
    Ok((true, None))
}

fn int(v: &OJson) -> i64 {
    match v {
        OJson::Int(i) => *i as i64,
        _ => 0,
    }
}

/// Translate an argument object by one adapter mapping; keys the mapping does not consume stay.
fn map_args(spec: &OJson, args: &OJson) -> OJson {
    let mut out = OJson::Obj(vec![]);
    let mut used: Vec<String> = vec![];
    for (key, m) in spec.get("args").entries() {
        let keys: Vec<String> = match m.get_opt("from") {
            Some(f) => strs(f).into_iter().map(String::from).collect(),
            None => vec![key.clone()],
        };
        used.extend(keys.iter().cloned());
        let mut val = keys
            .iter()
            .find_map(|k| args.get_opt(k))
            .cloned()
            .unwrap_or(OJson::Null);
        if val.is_null() {
            val = match m.get_opt("const") {
                Some(c) => c.clone(),
                None => m.get("default").clone(),
            };
        }
        if val.is_null() {
            continue;
        }
        if let OJson::Str(s) = &val {
            let mut s = s.clone();
            if m.get("as").as_str() == Some("basename") {
                s = s.rsplit('/').next().unwrap_or("").to_string();
            }
            if let Some(p) = m.get("prefix").as_str() {
                s = format!("{p}{s}");
            }
            val = OJson::Str(s);
        }
        out.set(key, val);
    }
    for (k, v) in args.entries() {
        if !used.contains(k) {
            out.set(k, v.clone());
        }
    }
    out
}

fn target<'a>(cap: &str, ad: &'a OJson, classes: &'a OJson) -> Result<&'a OJson, String> {
    classes
        .get_opt(cap)
        .or_else(|| ad.get("native_reads").get_opt(cap))
        .ok_or_else(|| format!("capability {cap} has no counterpart"))
}

/// `"sha256:" + sha256(json.dumps(obj, sort_keys=True, separators=(",", ":")))`
pub fn py_digest(v: &OJson) -> String {
    let text = dumps(v, Style::SORTED_COMPACT);
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

fn delivery(cap: &str, rid: &str, case: &OJson, table: &OJson) -> OJson {
    let lab = table.get("delivered_reads").get(cap);
    let stored = case.get("mock_data").get(cap);
    let content = if stored.truthy() {
        stored
            .get_opt("data")
            .cloned()
            .unwrap_or_else(|| format!("mock-delivered:{cap}").into())
    } else {
        format!("mock-delivered:{cap}").into()
    };
    obj([(
        "host_input",
        obj([(
            "input",
            obj([
                ("input_id", format!("in-delivered-{rid}").into()),
                ("content_kind", lab.get("content_kind").clone()),
                ("trust", lab.get("trust").clone()),
                (
                    "source",
                    obj([("kind", "runtime".into()), ("id", "host-delivery".into())]),
                ),
                ("origin", format!("runtime:delivery:{cap}").into()),
                ("content_digest", py_digest(&content).into()),
                ("trace_id", case.get("trace_id").clone()),
                ("parent_id", OJson::Null),
                ("derived_from", OJson::Arr(vec![])),
            ]),
        )]),
    )])
}

fn py_int_text(v: &OJson) -> String {
    match v {
        OJson::Int(i) => i.to_string(),
        OJson::Float(f) => (f.trunc() as i128).to_string(),
        OJson::Bool(b) => (*b as i32).to_string(),
        other => dumps(other, Style::PLAIN),
    }
}

fn is_delivered(cap: &str, ad: &OJson, classes: &OJson, table: &OJson) -> bool {
    table.get("delivered_reads").has(cap) && !classes.has(cap) && !ad.get("native_reads").has(cap)
}

/// Translate one call step. Calls the adapter lacks as read tools are dropped and queued as
/// host-registered deliveries, appended to `plan` after the step itself.
fn xlate_step(
    step: &OJson,
    case: &OJson,
    ad: &OJson,
    classes: &OJson,
    table: &OJson,
    plan: &mut Vec<OJson>,
) -> Result<(), String> {
    let mut deliveries = vec![];
    let mut s = step.clone();
    if s.has("envelope") {
        let env = s.get_mut("envelope").unwrap();
        let p = env.get_mut("payload").ok_or("envelope without payload")?;
        let name = p.get("tool").get("name").as_str().unwrap_or("").to_string();
        let spec = target(&name, ad, classes)?;
        let new_name = spec.get("name").clone();
        if let Some(tool) = p.get_mut("tool") {
            tool.set("name", new_name.clone());
        }
        if let Some(prov) = p.get_mut("provenance") {
            if prov.is_obj() {
                prov.set("raw_name", new_name);
            }
        }
        let args = p.get("arguments");
        let args = if args.truthy() {
            args.clone()
        } else {
            OJson::Obj(vec![])
        };
        p.set("arguments", map_args(spec, &args));
        env.set(
            "destination",
            obj([
                ("kind", "runtime".into()),
                ("id", ad.get("runtime_id").clone()),
            ]),
        );
        plan.push(s);
        return Ok(());
    }
    let turn = py_int_text(step.get("turn"));
    if s.get("dialect").as_str() == Some("openai") {
        let mut kept = vec![];
        for (i, c) in s.get("input").get("tool_calls").as_arr().iter().enumerate() {
            let mut c = c.clone();
            let cap = c
                .get("function")
                .get("name")
                .as_str()
                .unwrap_or("")
                .to_string();
            if is_delivered(&cap, ad, classes, table) {
                let id = c.get("id");
                let rid = if id.truthy() {
                    id.as_str().map(String::from).unwrap_or_default()
                } else {
                    format!("t{turn}c{i}")
                };
                deliveries.push(delivery(&cap, &rid, case, table));
                continue;
            }
            let spec = target(&cap, ad, classes)?;
            let f = c.get_mut("function").ok_or("tool call without function")?;
            f.set("name", spec.get("name").clone());
            let parsed = f
                .get("arguments")
                .as_str()
                .and_then(|t| OJson::parse(t).ok());
            if let Some(args @ OJson::Obj(_)) = parsed {
                f.set(
                    "arguments",
                    dumps(&map_args(spec, &args), Style::PLAIN).into(),
                );
            }
            kept.push(c);
        }
        let input = s.get_mut("input").ok_or("openai step without input")?;
        input.set(
            "tool_calls",
            if kept.is_empty() {
                OJson::Null
            } else {
                OJson::Arr(kept)
            },
        );
    } else {
        let text = s.get("input").as_str().unwrap_or("").to_string();
        let trace = s_trace(case);
        let mut idx = 0usize;
        let mut err = None;
        let params = param();
        let replaced = qwen().replace_all(&text, |m: &Captures| {
            let (cap, i) = (m[1].to_string(), idx);
            idx += 1;
            if is_delivered(&cap, ad, classes, table) {
                deliveries.push(delivery(
                    &cap,
                    &format!("{trace}:t{turn}:c{i}"),
                    case,
                    table,
                ));
                return String::new();
            }
            let spec = match target(&cap, ad, classes) {
                Ok(s) => s,
                Err(e) => {
                    err = Some(e);
                    return String::new();
                }
            };
            let mut found = OJson::Obj(vec![]);
            for p in params.captures_iter(&m[2]) {
                found.set(&p[1], p[2].to_string().into());
            }
            let args = map_args(spec, &found);
            let body: String = args
                .entries()
                .iter()
                .map(|(k, v)| format!("<parameter={k}>\n{}\n</parameter>\n", py_str(v)))
                .collect();
            format!(
                "<tool_call>\n<function={}>\n{body}</function>\n</tool_call>",
                spec.get("name").as_str().unwrap_or("")
            )
        });
        if let Some(e) = err {
            return Err(e);
        }
        s.set("input", replaced.trim().to_string().into());
    }
    plan.push(s);
    plan.extend(deliveries);
    Ok(())
}

fn s_trace(case: &OJson) -> String {
    case.get("trace_id").as_str().unwrap_or("").to_string()
}

/// Python `"%s" % v` for the values a translated parameter can hold.
fn py_str(v: &OJson) -> String {
    match v {
        OJson::Str(s) => s.clone(),
        OJson::Null => "None".into(),
        OJson::Bool(true) => "True".into(),
        OJson::Bool(false) => "False".into(),
        other => dumps(other, Style::PLAIN),
    }
}

fn approval_id(a: &OJson) -> OJson {
    let id = a.get("approval_id").as_str().unwrap_or("").to_string();
    match id.strip_prefix("mock-approval-") {
        Some(rest) if !rest.is_empty() && !rest.contains('\n') => {
            obj([("minted_for", rest.to_string().into())])
        }
        _ => obj([("literal", id.into())]),
    }
}

/// The plan of one fixture for `adapter`.
pub fn translate(name: &str, case: &OJson, adapter: &str, table: &OJson) -> Result<OJson, String> {
    let ad = adapter_entry(table, adapter)?;
    let is_approval = name.starts_with("approval/");
    let classes = if strs(table.get("provider_failure_cases")).contains(&name) {
        ad.get("provider_failure_effects")
    } else if is_approval {
        ad.get("approval_effects")
    } else {
        ad.get("effects")
    };
    let mut plan = vec![];
    for step in case.get("steps").as_arr() {
        if step.has("approve") || step.has("cancel") {
            let kind = if step.has("approve") {
                "approve"
            } else {
                "cancel"
            };
            let mut a = step.get(kind).clone();
            if kind == "approve" {
                a.set("approval_id", approval_id(&a));
                if a.has("arguments") {
                    let spec = classes
                        .get_opt("delete_file")
                        .ok_or("approval arguments without a delete_file mapping")?;
                    let mapped = map_args(spec, a.get("arguments"));
                    a.set("arguments", mapped);
                }
            }
            plan.push(obj([(kind, a)]));
        } else if step.has("restart") {
            plan.push(obj([("restart", OJson::Bool(true))]));
        } else if step.has("envelope") || step.has("dialect") {
            xlate_step(step, case, ad, classes, table, &mut plan)?;
        } else {
            plan.push(obj([("host_input", step.clone())]));
        }
    }
    let mut out = obj([
        ("name", name.into()),
        ("case", case.get("case").clone()),
        (
            "kind",
            if is_approval { "approval" } else { "injection" }.into(),
        ),
        ("trace_id", case.get("trace_id").clone()),
        ("steps", OJson::Arr(plan)),
    ]);
    if is_approval {
        out.set(
            "expires_at",
            case.get("mock_approval").get("expires_at").clone(),
        );
        let exp = case.get("expected");
        out.set(
            "expected",
            obj([
                ("observed", exp.get("observed").clone()),
                (
                    "continuations",
                    exp.get_opt("continuations")
                        .cloned()
                        .unwrap_or(OJson::Arr(vec![])),
                ),
            ]),
        );
    } else {
        let t = case.get("injection").get("target");
        let cap = t.get("capability").as_str().unwrap_or("");
        let spec = classes
            .get_opt(cap)
            .ok_or_else(|| format!("{name}: target {cap} has no mapping"))?;
        out.set(
            "target",
            obj([
                ("capability", spec.get("name").clone()),
                ("argument_contains", t.get("argument_contains").clone()),
            ]),
        );
    }
    Ok(out)
}
