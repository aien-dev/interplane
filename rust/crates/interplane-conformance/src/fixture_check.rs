//! Validate the INTERPLANE conformance and dialect fixtures against `spec/schemas`.
//!
//! Checks:
//!   * every explicit envelope step validates against envelope.schema.json
//!   * every intent in dialects/fixtures validates against intent.schema.json
//!   * fixture shape, 40 required cases, filename == case slug
//!   * lifecycle/ fixtures: required set, shape, minting decisions valid against decision.schema.json
//!   * expected.runtime counts agree with the observed records
//!   * every digest fixture recomputes; InputRecord examples (one per source class) validate
//!   * injection and approval fixtures: shape, categories, and negative controls on mutants
//!   * gate I corpus accounting and gate N category coverage (bench/PROTOCOL-0.3.md 3.2)
//!   * negative control: a mutated envelope MUST fail validation
//!
//! [`run`] returns the summary lines and every failure; the `interplane-corpus validate` command
//! prints them and exits non-zero on any failure.
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use interplane_core::canonicalize;
use jsonschema::{Resource, Validator};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::ojson::{dumps, obj, OJson, Style};

/// What a run found: the summary lines (printed on success and failure) and the failures.
#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub errors: Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }

    /// The text the command prints: summary lines, then `PASS` or `FAIL (n)` with every failure.
    pub fn text(&self) -> String {
        let mut out: String = self.lines.iter().map(|l| format!("{l}\n")).collect();
        if self.errors.is_empty() {
            out.push_str("PASS\n");
        } else {
            out.push_str(&format!("FAIL ({})\n", self.errors.len()));
            for e in &self.errors {
                out.push_str(&format!(" - {e}\n"));
            }
        }
        out
    }
}

/// Every schema under `spec/schemas`, sorted by file name: `(file name, schema)`.
pub fn load_schemas(root: &Path) -> Result<Vec<(String, Value)>, String> {
    let dir = root.join("spec/schemas");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let text = std::fs::read_to_string(dir.join(&n)).map_err(|e| format!("{n}: {e}"))?;
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("{n}: {e}"))?;
            Ok((n, v))
        })
        .collect()
}

/// Check every schema against the JSON Schema 2020-12 meta-schema; one `ok <path>` line each.
pub fn self_check_schemas(root: &Path) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for (n, s) in load_schemas(root)? {
        jsonschema::meta::validate(&s).map_err(|e| format!("spec/schemas/{n}: {e}"))?;
        out.push(format!("ok spec/schemas/{n}"));
    }
    Ok(out)
}

struct Schemas {
    all: Vec<(String, Value)>,
}

impl Schemas {
    fn validator(&self, name: &str) -> Result<Validator, String> {
        let main = &self
            .all
            .iter()
            .find(|(n, _)| n == name)
            .ok_or_else(|| format!("schema {name} missing"))?
            .1;
        jsonschema::meta::validate(main).map_err(|e| format!("{name}: {e}"))?;
        let mut resources = vec![];
        for (_, s) in &self.all {
            if let Some(id) = s["$id"].as_str() {
                let r = Resource::from_contents(s.clone()).map_err(|e| e.to_string())?;
                resources.push((id.to_string(), r));
            }
        }
        jsonschema::options()
            .with_resources(resources.into_iter())
            .build(main)
            .map_err(|e| format!("{name}: {e}"))
    }
}

struct Ctx {
    errors: Vec<String>,
}

impl Ctx {
    fn fail(&mut self, m: String) {
        self.errors.push(m);
    }

    /// Validate `inst`; record at most three schema errors, ordered by instance path.
    fn check(&mut self, v: &Validator, inst: &OJson, where_: &str) -> bool {
        let value = inst.to_value();
        let mut errs: Vec<(Vec<String>, String)> = v
            .iter_errors(&value)
            .map(|e| {
                let path: Vec<String> = e
                    .instance_path
                    .to_string()
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                (path, e.to_string())
            })
            .collect();
        errs.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, msg) in errs.iter().take(3) {
            let msg: String = msg.chars().take(200).collect();
            self.fail(format!("{where_}: {msg} at /{}", path.join("/")));
        }
        errs.is_empty()
    }
}

fn valid(v: &Validator, inst: &OJson) -> bool {
    v.is_valid(&inst.to_value())
}

fn jcs(v: &OJson) -> String {
    canonicalize(&v.to_value())
}

fn sha_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// Python `repr` of a value, for the few messages that print one.
fn py_repr(v: &OJson) -> String {
    match v {
        OJson::Null => "None".into(),
        OJson::Bool(true) => "True".into(),
        OJson::Bool(false) => "False".into(),
        OJson::Str(s) => py_repr_str(s),
        other => dumps(other, Style::PLAIN),
    }
}

fn py_repr_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn py_list<S: AsRef<str>>(items: &[S]) -> String {
    let inner: Vec<String> = items.iter().map(|s| py_repr_str(s.as_ref())).collect();
    format!("[{}]", inner.join(", "))
}

fn sym_diff(a: &BTreeSet<String>, b: &BTreeSet<String>) -> Vec<String> {
    a.symmetric_difference(b).cloned().collect()
}

fn set_of(v: &OJson) -> BTreeSet<String> {
    v.keys().into_iter().map(String::from).collect()
}

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.into_iter().map(String::from).collect()
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn stem(p: &Path) -> String {
    let n = p.file_name().unwrap().to_string_lossy();
    n[..n.len() - 5].to_string()
}

fn count_true(obs: &[OJson], key: &str) -> i128 {
    obs.iter().filter(|o| o.get(key).truthy()).count() as i128
}

fn as_int(v: &OJson) -> Option<i128> {
    match v {
        OJson::Int(i) => Some(*i),
        _ => None,
    }
}

const REQUIRED: [&str; 40] = [
    "01-valid-tool-request",
    "02-malformed-arguments",
    "03-nonexistent-tool",
    "04-unknown-capability",
    "05-policy-denial",
    "06-explicit-approval",
    "07-successful-execution",
    "08-execution-error",
    "09-timeout",
    "10-multiple-sequential-calls",
    "11-model-continuation",
    "12-unsupported-dialect",
    "13-unsupported-version",
    "14-unknown-fields",
    "15-oversized-arguments",
    "16-duplicate-request-ids",
    "17-replay",
    "18-model-retry-after-denial",
    "19-valid-write-request",
    "20-missing-required-argument",
    "21-stale-capability-mapping",
    "22-untrusted-tool-result",
    "23-untrusted-memory-result",
    "24-expansion-requested-excluded",
    "25-expansion-refused-by-bound",
    "26-unrecognized-content-kind",
    "27-absent-null-unrecognized-trust",
    "28-input-ledger-continues",
    "29-input-derived-from",
    "30-input-forged-exposure",
    "31-source-class-workspace",
    "32-source-class-web",
    "33-source-class-memory",
    "34-source-class-document",
    "35-source-class-skill",
    "36-source-class-tool-output",
    "37-source-class-external-provider",
    "38-source-class-runtime-generated",
    "39-source-class-user-request",
    "40-source-class-model-generated",
];
const OBS_KEYS: [&str; 8] = [
    "request_id",
    "stage",
    "decision",
    "status",
    "error_code",
    "decide_invoked",
    "execute_invoked",
    "result_digest",
];
const STATES: [&str; 10] = [
    "PROPOSED",
    "REJECTED",
    "MAPPED",
    "DENIED",
    "REQUIRES_APPROVAL",
    "AUTHORIZED",
    "EXECUTING",
    "SUCCEEDED",
    "FAILED",
    "TIMED_OUT",
];
const INJECTION: [(&str, &str); 44] = [
    ("01-workspace-write", "C01"),
    ("02-tool-output-markup", "C02"),
    ("03-forged-approval-in-arguments", "C07"),
    ("04-workspace-xml-write", "C01"),
    ("05-workspace-json-delete", "C01"),
    ("06-tool-output-json-email", "C02"),
    ("07-tool-output-plain-write", "C02"),
    ("08-tool-output-nested-result", "C02"),
    ("09-tool-output-error-message", "C02"),
    ("10-memory-xml-write", "C03"),
    ("11-memory-json-email", "C03"),
    ("12-memory-plain-delete", "C03"),
    ("13-web-xml-delete", "C04"),
    ("14-web-json-email", "C04"),
    ("15-web-plain-write", "C04"),
    ("16-document-xml-write", "C05"),
    ("17-document-json-email", "C05"),
    ("18-document-plain-delete", "C05"),
    ("19-document-embedded-markup", "C05"),
    ("20-impersonate-system-prompt", "C06"),
    ("21-impersonate-runtime", "C06"),
    ("22-impersonate-user", "C06"),
    ("23-forged-approval-in-extensions", "C07"),
    ("24-forged-provenance-in-arguments", "C07"),
    ("25-forged-exposure-in-envelope", "C07"),
    ("26-fake-runtime-approval", "C08"),
    ("27-fake-tool-result", "C08"),
    ("28-fake-policy-change", "C08"),
    ("29-retry-new-request-id", "C09"),
    ("30-retry-fabricated-approval", "C09"),
    ("31-retry-after-requires-approval", "C09"),
    ("32-read-result-steers-to-write", "C11"),
    ("33-tool-description-steers-to-email", "C11"),
    ("34-low-privilege-chain-to-write", "C11"),
    ("35-decision-cites-other-request", "C12"),
    ("36-result-cites-other-request", "C12"),
    ("37-stale-catalog-digest", "C12"),
    ("38-duplicate-advertised-name", "C12"),
    ("39-decide-raises", "C13"),
    ("40-execute-raises", "C13"),
    ("41-denied-call-interleaved", "C13"),
    ("42-rejected-call-interleaved", "C13"),
    ("43-web-effect-held-for-approval", "C04"),
    ("44-memory-effect-held-for-approval", "C03"),
];
/// Gate I minimum cases per category (bench/PROTOCOL-0.3.md 3.2).
const MIN: [(&str, usize); 13] = [
    ("C01", 3),
    ("C02", 5),
    ("C03", 3),
    ("C04", 3),
    ("C05", 4),
    ("C06", 3),
    ("C07", 4),
    ("C08", 3),
    ("C09", 3),
    ("C10", 10),
    ("C11", 3),
    ("C12", 4),
    ("C13", 4),
];
const INPUT_CLASSES: [(&str, &str); 9] = [
    ("workspace", "workspace_content"),
    ("web", "web_content"),
    ("memory", "memory"),
    ("document", "document"),
    ("skill", "skill"),
    ("tool-output", "tool_result"),
    ("external-provider", "external_provider"),
    ("runtime-generated", "runtime_instruction"),
    ("user-request", "user_request"),
];

fn injection_category(name: &str) -> Option<&'static str> {
    INJECTION.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

fn injection_problems(f: &OJson, name: &str) -> Vec<String> {
    let mut bad = vec![];
    let inj = f.get("injection");
    if !inj.is_obj() || set_of(inj) != set(["category", "canary", "target"]) {
        return vec![format!("{name}: injection block keys")];
    }
    let tgt = inj.get("target");
    let category = inj.get("category");
    let known = category
        .as_str()
        .is_some_and(|c| MIN.iter().any(|(k, _)| *k == c));
    if !known {
        bad.push(format!("{name}: unknown category {}", py_repr(category)));
    }
    if !inj.get("canary").as_str().is_some_and(|s| !s.is_empty()) {
        bad.push(format!("{name}: empty canary"));
    }
    if !tgt.is_obj()
        || set_of(tgt) != set(["capability", "argument_contains"])
        || !tgt
            .entries()
            .iter()
            .all(|(_, v)| v.as_str().is_some_and(|s| !s.is_empty()))
    {
        bad.push(format!(
            "{name}: injection.target needs capability and argument_contains"
        ));
        return bad;
    }
    let (cap, needle) = (
        tgt.get("capability").as_str().unwrap(),
        tgt.get("argument_contains").as_str().unwrap(),
    );
    let mut obeyed = false;
    for s in f.get("steps").as_arr() {
        let host_keys = ["approval", "approve", "continuation", "decision"];
        if host_keys.iter().any(|k| s.has(k))
            || s.get("envelope").get("payload").get("kind").as_str() == Some("decision")
        {
            bad.push(format!(
                "{name}: an injection fixture has no host approval step"
            ));
        }
        if s.has("dialect") || s.has("envelope") {
            let part = if s.has("dialect") {
                s.get("input")
            } else {
                s.get("envelope").get("payload")
            };
            let text = dumps(part, Style::PLAIN);
            if text.contains(cap) && text.contains(needle) {
                obeyed = true;
            }
        }
        if s.has("content") {
            if !s.has("input") || s.has("dialect") {
                bad.push(format!("{name}: content only on host input steps"));
            } else {
                let content = s.get("content").as_str().unwrap_or("");
                let want = format!("sha256:{}", sha_hex(content.as_bytes()));
                if s.get("input").get("content_digest").as_str() != Some(want.as_str()) {
                    bad.push(format!(
                        "{name}: input content does not match content_digest"
                    ));
                }
            }
        }
    }
    if !obeyed {
        bad.push(format!(
            "{name}: no model step emits the injected intent (target capability with the canary)"
        ));
    }
    let steps = f.get_opt("steps").cloned().unwrap_or(OJson::Arr(vec![]));
    if !dumps(&steps, Style::PLAIN).contains(inj.get("canary").as_str().unwrap_or("")) {
        bad.push(format!("{name}: canary appears in no step"));
    }
    bad
}

fn with_steps(f: &OJson, steps: Vec<OJson>) -> OJson {
    let mut m = f.clone();
    m.set("steps", OJson::Arr(steps));
    m
}

/// Run every check against the checkout at `root`.
pub fn run(root: &Path) -> Result<Report, String> {
    let schemas = Schemas {
        all: load_schemas(root)?,
    };
    let v_env = schemas.validator("envelope.schema.json")?;
    let v_intent = schemas.validator("intent.schema.json")?;
    let v_sel = schemas.validator("selection.schema.json")?;
    let v_input = schemas.validator("input.schema.json")?;
    let v_dec = schemas.validator("decision.schema.json")?;
    let mut cx = Ctx { errors: vec![] };
    let fix = root.join("conformance/fixtures");

    // ---- conformance fixtures
    let (mut n_env, mut n_cases) = (0usize, 0usize);
    let mut neg_done = false;
    let mut seen = BTreeSet::new();
    let required: BTreeSet<String> = REQUIRED.iter().map(|s| s.to_string()).collect();
    for p in json_files(&fix) {
        let f = OJson::read(&p)?;
        let name = stem(&p);
        n_cases += 1;
        seen.insert(name.clone());
        if f.get("case").as_str() != Some(name.as_str()) {
            cx.fail(format!(
                "{name}: case slug {} != filename",
                py_repr(f.get("case"))
            ));
        }
        for k in ["description", "limits", "trace_id", "steps", "expected"] {
            if !f.has(k) {
                cx.fail(format!("{name}: missing {k}"));
            }
        }
        let exp = f.get("expected");
        let obs = exp.get("observed").as_arr();
        for (i, o) in obs.iter().enumerate() {
            if o.keys() != OBS_KEYS {
                let keys: Vec<&str> = o.keys();
                cx.fail(format!(
                    "{name}: observed[{i}] keys/order {}",
                    py_list(&keys)
                ));
            }
            if !o.get("result_digest").is_null() {
                cx.fail(format!("{name}: observed[{i}] result_digest must be null"));
            }
        }
        let rt = exp.get("runtime");
        if as_int(rt.get("decide_calls")) != Some(count_true(obs, "decide_invoked")) {
            cx.fail(format!("{name}: decide_calls mismatch"));
        }
        if as_int(rt.get("execute_calls")) != Some(count_true(obs, "execute_invoked")) {
            cx.fail(format!("{name}: execute_calls mismatch"));
        }
        for s in f.get("steps").as_arr() {
            if s.has("envelope") {
                n_env += 1;
                let where_ = format!("{name} step {} envelope", py_repr(s.get("turn")));
                if cx.check(&v_env, s.get("envelope"), &where_) && !neg_done {
                    let mut bad = s.get("envelope").clone();
                    if let Some(tool) = bad.get_mut("payload").and_then(|p| p.get_mut("tool")) {
                        tool.set("unexpected", OJson::Int(1));
                    }
                    if valid(&v_env, &bad) {
                        cx.fail("negative control: mutated envelope (extra key in tool) unexpectedly valid".into());
                    }
                    let mut bad = s.get("envelope").clone();
                    if let OJson::Obj(m) = &mut bad {
                        m.retain(|(k, _)| k != "trace_id");
                    }
                    if valid(&v_env, &bad) {
                        cx.fail(
                            "negative control: envelope without trace_id unexpectedly valid".into(),
                        );
                    }
                    neg_done = true;
                }
            } else if s.has("dialect") && s.has("input") {
            } else if s.has("input") && !s.has("dialect") {
                // host-only input registration (0.3 cut P3)
                cx.check(&v_input, s.get("input"), &format!("{name} input step"));
            } else if s.has("expand") {
                if !f.has("selection") {
                    cx.fail(format!("{name}: expand step without a selection block"));
                }
                let kind = s.get("expand").get("evidence").get("kind").as_str();
                if !matches!(kind, Some("requested_excluded") | Some("discovery_hit")) {
                    cx.fail(format!("{name}: bad evidence kind"));
                }
            } else {
                cx.fail(format!(
                    "{name}: step {} has neither envelope nor dialect+input",
                    py_repr(s.get("turn"))
                ));
            }
        }
        for rec in exp.get("inputs").as_arr() {
            if rec.has("content_digest") {
                cx.check(&v_input, rec, &format!("{name} expected.inputs"));
            }
        }
        for e in exp.get("exposure").as_arr() {
            if set_of(e) != set(["request_id", "inputs", "floor"]) {
                cx.fail(format!(
                    "{name}: malformed expected.exposure {}",
                    py_dict_repr(e)
                ));
            }
        }
        let obs_ids: Vec<&OJson> = obs.iter().map(|o| o.get("request_id")).collect();
        for c in exp.get("result_checks").as_arr() {
            let mut ks = set_of(c);
            ks.remove("round_trip");
            let rt_ok = match c.get_opt("round_trip") {
                None => true,
                Some(v) => *v == OJson::Bool(true),
            };
            if ks != set(["request_id", "path", "equals"])
                || !rt_ok
                || !obs_ids.contains(&c.get("request_id"))
            {
                cx.fail(format!(
                    "{name}: malformed result_check {}",
                    py_dict_repr(c)
                ));
            }
        }
        let pre = &name[..2.min(name.len())];
        if f.has("selection") != matches!(pre, "24" | "25") {
            cx.fail(format!("{name}: selection block only in cases 24-25"));
        }
        if f.has("selection") {
            let n_exp = f
                .get("steps")
                .as_arr()
                .iter()
                .filter(|s| s.has("expand"))
                .count();
            let sels = exp.get("selections").as_arr();
            if sels.len() != n_exp {
                cx.fail(format!(
                    "{name}: expected.selections must have one entry per expand step"
                ));
            }
            for (i, sel) in sels.iter().enumerate() {
                cx.check(&v_sel, sel, &format!("{name} selections[{i}]"));
                let body = OJson::Obj(
                    sel.entries()
                        .iter()
                        .filter(|(k, _)| k != "selection_digest" && k != "measure")
                        .cloned()
                        .collect(),
                );
                let want = format!("sha256:{}", sha_hex(jcs(&body).as_bytes()));
                if sel.get("selection_digest").as_str() != Some(want.as_str()) {
                    cx.fail(format!(
                        "{name}: selections[{i}] selection_digest does not recompute"
                    ));
                }
                if i > 0 && sel.get("parent_digest") != sels[i - 1].get("selection_digest") {
                    cx.fail(format!("{name}: parent_digest chain broken at {i}"));
                }
            }
        }
        if f.has("mapping_table") != name.starts_with("21") {
            cx.fail(format!("{name}: mapping_table present only in case 21"));
        }
        if f.has("mock_provenance") != matches!(pre, "26" | "27") {
            cx.fail(format!(
                "{name}: mock_provenance present only in cases 26-27"
            ));
        }
        for (cap, o) in f.get("mock_provenance").entries() {
            if !set_of(o).is_subset(&set(["content_kind", "trust", "trusted"])) {
                let keys: Vec<String> = set_of(o).into_iter().collect();
                cx.fail(format!(
                    "{name}: mock_provenance.{cap} keys {}",
                    py_list(&keys)
                ));
            }
        }
        let checks = exp.get("result_checks").as_arr();
        if matches!(pre, "26" | "27") {
            let pair = |c: &OJson| {
                (
                    dumps(c.get("request_id"), Style::PLAIN),
                    dumps(c.get("path"), Style::PLAIN),
                )
            };
            let rt_paths: BTreeSet<_> = checks
                .iter()
                .filter(|c| c.get("round_trip").truthy())
                .map(pair)
                .collect();
            let plain: BTreeSet<_> = checks
                .iter()
                .filter(|c| !c.get("round_trip").truthy())
                .map(pair)
                .collect();
            if plain.is_empty() || rt_paths != plain {
                cx.fail(format!(
                    "{name}: every provenance check must also be checked after a JCS round trip"
                ));
            }
            for c in checks {
                if c.get("path").as_str() == Some("provenance.trusted")
                    && *c.get("equals") == OJson::Bool(true)
                {
                    cx.fail(format!("{name}: trusted must never be true here"));
                }
            }
        }
        if name.starts_with("21") && f.get("mapping_table").as_str() != Some("mock-table-stale") {
            cx.fail("21: mapping_table must be mock-table-stale".into());
        }
        if matches!(pre, "19" | "22" | "23" | "20" | "21") {
            let want = match pre {
                "19" | "22" | "23" => ("SUCCEEDED", "ok"),
                _ => ("REJECTED", "rejected"),
            };
            let first = obs.first().cloned().unwrap_or(OJson::Null);
            let got = (first.get("stage").clone(), first.get("status").clone());
            if got.0.as_str() != Some(want.0) || got.1.as_str() != Some(want.1) {
                cx.fail(format!(
                    "{name}: unexpected stage/status ({}, {})",
                    py_repr(&got.0),
                    py_repr(&got.1)
                ));
            }
            let pairs: HashSet<(String, String)> = checks
                .iter()
                .map(|c| {
                    (
                        c.get("path").as_str().unwrap_or("").to_string(),
                        dumps(c.get("equals"), Style::PLAIN),
                    )
                })
                .collect();
            let need: &[(&str, &str)] = match pre {
                "19" => &[("data.appended", "10")],
                "20" => &[("error.message", "\"missing required argument: path\"")],
                "21" => &[(
                    "error.message",
                    "\"mapping table catalog digest does not match runtime catalog\"",
                )],
                "22" => &[
                    ("provenance.trust", "\"external_untrusted\""),
                    ("provenance.content_kind", "\"web_content\""),
                ],
                _ => &[
                    ("provenance.trust", "\"workspace_untrusted\""),
                    ("provenance.content_kind", "\"memory\""),
                ],
            };
            for (p, e) in need {
                if !pairs.contains(&(p.to_string(), e.to_string())) {
                    cx.fail(format!(
                        "{name}: missing result_check ({}, {})",
                        py_repr_str(p),
                        py_repr_str(e)
                    ));
                }
            }
        }
        if name.starts_with("22") {
            let outcomes: Vec<&OJson> = exp
                .get("turns")
                .as_arr()
                .iter()
                .map(|t| t.get("outcome"))
                .collect();
            let shape_ok = f
                .get("steps")
                .as_arr()
                .get(1)
                .is_some_and(|s| s.get("continues").truthy())
                && outcomes == [&OJson::from("tool_request"), &OJson::from("no_tool")];
            if !shape_ok {
                cx.fail("22: continuation shape".into());
            }
            if obs.len() != 1 {
                cx.fail("22: the continuing turn must add no records".into());
            }
            let content = checks
                .iter()
                .find(|c| c.get("path").as_str() == Some("data.content"))
                .and_then(|c| c.get("equals").as_str())
                .unwrap_or("");
            if !content.contains("<tool_call>") {
                cx.fail("22: pinned content must carry the markup".into());
            }
        }
        // size checks for the oversized case
        if name.starts_with("15") {
            let text = f.get("steps").as_arr()[0]
                .get("input")
                .get("tool_calls")
                .as_arr()[0]
                .get("function")
                .get("arguments")
                .as_str()
                .unwrap_or("")
                .to_string();
            let a = OJson::parse(&text)?;
            let content = a.get("content").as_str().unwrap_or("");
            if content.chars().count() != 70000 || content.chars().any(|c| c != 'a') {
                cx.fail("15: content must be 70000 x 'a'".into());
            }
            if jcs(&a).len() <= 65536 {
                cx.fail("15: arguments not oversized".into());
            }
        }
    }
    if seen != required {
        cx.fail(format!(
            "required cases missing/extra: {}",
            py_list(&sym_diff(&seen, &required))
        ));
    }
    if !neg_done {
        cx.fail("negative control never ran".into());
    }

    // ---- lifecycle fixtures (driven against the Lifecycle directly, no pipeline)
    let lifecycle = set([
        "01-forged-approval-id",
        "02-empty-approval-id",
        "03-approval-id-from-other-request",
        "04-runtime-minted-approval-id",
        "05-continuation-non-authorized-value",
    ]);
    let mut n_life = 0;
    let mut lseen = BTreeSet::new();
    for p in json_files(&fix.join("lifecycle")) {
        let f = OJson::read(&p)?;
        let name = stem(&p);
        n_life += 1;
        lseen.insert(name.clone());
        if f.get("case").as_str() != Some(format!("lifecycle/{name}").as_str()) {
            cx.fail(format!(
                "lifecycle/{name}: case slug {} != filename",
                py_repr(f.get("case"))
            ));
        }
        if set_of(&f) != set(["case", "description", "steps", "expected"]) {
            let keys: Vec<String> = set_of(&f).into_iter().collect();
            cx.fail(format!("lifecycle/{name}: keys {}", py_list(&keys)));
        }
        let (steps, exp) = (f.get("steps").as_arr(), f.get("expected").as_arr());
        if steps.len() != exp.len() {
            cx.fail(format!("lifecycle/{name}: one expected row per step"));
        }
        let mut minted: Vec<OJson> = vec![];
        for (i, (s, e)) in steps.iter().zip(exp).enumerate() {
            let d = s.get("decision");
            if e.keys() != ["request_id", "state", "refused"]
                || !e
                    .get("state")
                    .as_str()
                    .is_some_and(|st| STATES.contains(&st))
                || e.get("request_id") != s.get("request_id")
            {
                cx.fail(format!("lifecycle/{name}: expected[{i}] shape"));
            }
            if !minted.contains(s.get("request_id")) {
                // first decision on a request is the runtime's own: requires_approval with a minted id
                let d = if s.has("decision") {
                    d.clone()
                } else {
                    OJson::Obj(vec![])
                };
                cx.check(
                    &v_dec,
                    &d,
                    &format!("lifecycle/{name} step {i} minting decision"),
                );
                if d.get("decision").as_str() != Some("requires_approval")
                    || d.get("request_id") != s.get("request_id")
                {
                    cx.fail(format!("lifecycle/{name}: step {i} must mint"));
                }
                minted.push(s.get("request_id").clone());
            } else if d.get("request_id") != s.get("request_id") {
                cx.fail(format!(
                    "lifecycle/{name}: step {i} continuation must cite its own request_id"
                ));
            }
        }
    }
    if lseen != lifecycle {
        cx.fail(format!(
            "lifecycle cases missing/extra: {}",
            py_list(&sym_diff(&lseen, &lifecycle))
        ));
    }

    // ---- injection fixtures (0.3 cut I1, bench/PROTOCOL-0.3.md section 3)
    let mut n_inj = 0;
    let mut iseen = BTreeSet::new();
    for p in json_files(&fix.join("injection")) {
        let f = OJson::read(&p)?;
        let iname = stem(&p);
        n_inj += 1;
        iseen.insert(iname.clone());
        if f.get("case").as_str() != Some(format!("injection-{iname}").as_str()) {
            cx.fail(format!(
                "injection/{iname}: case slug {}",
                py_repr(f.get("case"))
            ));
        }
        for k in [
            "description",
            "limits",
            "trace_id",
            "steps",
            "expected",
            "injection",
        ] {
            if !f.has(k) {
                cx.fail(format!("injection/{iname}: missing {k}"));
            }
        }
        for m in injection_problems(&f, &format!("injection/{iname}")) {
            cx.fail(m);
        }
        if let Some(cat) = injection_category(&iname) {
            if f.get("injection").get("category").as_str() != Some(cat) {
                cx.fail(format!("injection/{iname}: category must be {cat}"));
            }
        }
        let exp = f.get("expected");
        let obs = exp.get("observed").as_arr();
        for (i, o) in obs.iter().enumerate() {
            if o.keys() != OBS_KEYS || !o.get("result_digest").is_null() {
                cx.fail(format!("injection/{iname}: observed[{i}] shape"));
            }
        }
        let rt = exp.get("runtime");
        if as_int(rt.get("decide_calls")) != Some(count_true(obs, "decide_invoked"))
            || as_int(rt.get("execute_calls")) != Some(count_true(obs, "execute_invoked"))
        {
            cx.fail(format!("injection/{iname}: runtime counts mismatch"));
        }
        for s in f.get("steps").as_arr() {
            if s.has("envelope") {
                cx.check(
                    &v_env,
                    s.get("envelope"),
                    &format!("injection/{iname} envelope"),
                );
            } else if s.has("input") && !s.has("dialect") {
                cx.check(
                    &v_input,
                    s.get("input"),
                    &format!("injection/{iname} input step"),
                );
            } else if s.has("mock")
                && set_of(s.get("mock")) == set(["catalog_digest"])
                && s.entries().len() == 1
            {
            } else if !(s.has("dialect") && s.has("input")) {
                cx.fail(format!("injection/{iname}: unsupported step"));
            }
        }
        let fault = f.get("mock_fault");
        let fault_keys = set([
            "decide_raises",
            "execute_raises",
            "decision_request_id",
            "result_request_id",
            "duplicate_capability",
        ]);
        if !set_of(fault).is_subset(&fault_keys) {
            let keys: Vec<String> = set_of(fault).into_iter().collect();
            cx.fail(format!(
                "injection/{iname}: mock_fault keys {}",
                py_list(&keys)
            ));
        }
        for d in fault.get("duplicate_capability").as_arr() {
            let ks = set_of(d);
            let ok = d.is_obj()
                && set(["name", "description", "parameters"]).is_subset(&ks)
                && ks.is_subset(&set(["name", "description", "parameters", "domains"]));
            if !ok {
                cx.fail(format!("injection/{iname}: duplicate_capability shape"));
            }
        }
        for e in exp.get("exposure").as_arr() {
            if set_of(e) != set(["request_id", "inputs", "floor"]) {
                cx.fail(format!("injection/{iname}: malformed expected.exposure"));
            }
        }
        if !exp.get("exposure").truthy() {
            cx.fail(format!(
                "injection/{iname}: must assert the exposure the runtime saw"
            ));
        }
        // negative control: the same fixture with a host approval step must be rejected
        let mut steps = f.get("steps").as_arr().to_vec();
        steps.push(obj([("approval", obj([("approval_id", "x".into())]))]));
        if injection_problems(&with_steps(&f, steps), "mutant").is_empty() {
            cx.fail("negative control: injection fixture with an approval step accepted".into());
        }
        let model_free: Vec<OJson> = f
            .get("steps")
            .as_arr()
            .iter()
            .filter(|s| !s.has("dialect") && !s.has("envelope"))
            .cloned()
            .collect();
        if injection_problems(&with_steps(&f, model_free), "mutant").is_empty() {
            cx.fail(
                "negative control: injection fixture without an obeying model step accepted".into(),
            );
        }
    }
    let all_inj: BTreeSet<String> = INJECTION.iter().map(|(n, _)| n.to_string()).collect();
    if iseen != all_inj {
        cx.fail(format!(
            "injection cases missing/extra: {}",
            py_list(&sym_diff(&iseen, &all_inj))
        ));
    }

    // ---- approval fixtures (0.3 cut A2, bench/PROTOCOL-0.3.md section 3.3)
    let approval: BTreeSet<String> = (1..18).map(|i| format!("{i:02}")).collect();
    let shape = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();
    let shape_ok = |v: &OJson| v.as_str().is_some_and(|s| shape.is_match(s));
    let cont_keys = [
        "step",
        "kind",
        "request_id",
        "outcome",
        "stage",
        "reason",
        "message",
    ];
    let mut n_app = 0;
    let mut aseen = BTreeSet::new();
    let approval_files = json_files(&fix.join("approval"));
    for p in &approval_files {
        let f = OJson::read(p)?;
        let aname = stem(p);
        n_app += 1;
        aseen.insert(aname[..2].to_string());
        let w = format!("approval/{aname}");
        if f.get("case").as_str()
            != Some(format!("approval-A{}-{}", &aname[..2], &aname[3..]).as_str())
        {
            cx.fail(format!("{w}: case slug {}", py_repr(f.get("case"))));
        }
        for k in ["description", "limits", "trace_id", "steps", "expected"] {
            if !f.has(k) {
                cx.fail(format!("{w}: missing {k}"));
            }
        }
        let exp = f.get("expected");
        let obs = exp.get("observed").as_arr();
        for (i, o) in obs.iter().enumerate() {
            if o.keys() != OBS_KEYS || !o.get("result_digest").is_null() {
                cx.fail(format!("{w}: observed[{i}] shape"));
            }
        }
        let rt = exp.get("runtime");
        if as_int(rt.get("decide_calls")) != Some(count_true(obs, "decide_invoked"))
            || as_int(rt.get("execute_calls")) != Some(count_true(obs, "execute_invoked"))
        {
            cx.fail(format!("{w}: runtime counts mismatch"));
        }
        if f.has("mock_approval") {
            let ma = f.get("mock_approval");
            if set_of(ma) != set(["expires_at"]) || !shape_ok(ma.get("expires_at")) {
                cx.fail(format!("{w}: mock_approval shape"));
            }
        }
        let steps = f.get("steps").as_arr();
        let host: Vec<(usize, &str)> = steps
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                ["approve", "cancel"]
                    .into_iter()
                    .filter(move |k| s.has(k))
                    .map(move |k| (i, k))
            })
            .collect();
        let conts_v = exp.get("continuations");
        let mut conts: &[OJson] = conts_v.as_arr();
        if !matches!(conts_v, OJson::Arr(_)) || conts.len() != host.len() {
            cx.fail(format!(
                "{w}: one continuations row per approve/cancel step"
            ));
            conts = &[];
        }
        for (row, (i, k)) in conts.iter().zip(&host) {
            let st = steps[*i].get(k);
            if row.keys() != cont_keys
                || *row.get("step") != OJson::Int(*i as i128)
                || row.get("kind").as_str() != Some(*k)
                || row.get("request_id") != st.get("request_id")
            {
                cx.fail(format!("{w}: continuations row for step {i}"));
            } else if row.get("outcome").as_str() == Some("refused") {
                let stage = row.get("stage");
                if row.get("reason").as_str() != Some("no_pending_approval")
                    || !row.get("message").is_null()
                    || (!stage.is_null() && !stage.as_str().is_some_and(|s| STATES.contains(&s)))
                {
                    cx.fail(format!("{w}: refused row shape (step {i})"));
                }
            } else if row.get("outcome").as_str() != Some("resolved")
                || !row.get("reason").is_null()
                || !row
                    .get("stage")
                    .as_str()
                    .is_some_and(|s| STATES.contains(&s))
            {
                cx.fail(format!("{w}: resolved row shape (step {i})"));
            }
        }
        for (i, s) in steps.iter().enumerate() {
            if s.has("envelope") {
                cx.check(&v_env, s.get("envelope"), &format!("{w} envelope"));
            } else if s.has("dialect") && s.has("input") {
            } else if s.has("approve") {
                let a = s.get("approve");
                let ks = set_of(a);
                let allowed = set([
                    "request_id",
                    "approval_id",
                    "decision",
                    "decision_request_id",
                    "arguments",
                    "now",
                    "trace_id",
                ]);
                if !set(["request_id", "approval_id"]).is_subset(&ks) || !ks.is_subset(&allowed) {
                    cx.fail(format!("{w}: approve step keys"));
                }
                if a.has("now") && !shape_ok(a.get("now")) {
                    cx.fail(format!("{w}: approve.now shape"));
                }
            } else if s.has("cancel") {
                let c = s.get("cancel");
                if !c.has("request_id") || !set_of(c).is_subset(&set(["request_id", "trace_id"])) {
                    cx.fail(format!("{w}: cancel step keys"));
                }
            } else if !(s.entries().len() == 1
                && (*s.get("restart") == OJson::Bool(true)
                    || (s.has("mock") && set_of(s.get("mock")) == set(["catalog_digest"]))))
            {
                cx.fail(format!("{w}: unsupported step {i}"));
            }
        }
        // only the host side issues approve/cancel: a model step or envelope must never carry one
        for s in steps {
            if (s.has("envelope") || s.has("dialect"))
                && ["approve", "cancel", "restart", "mock"]
                    .iter()
                    .any(|k| s.has(k))
            {
                cx.fail(format!("{w}: host step mixed into a model step"));
            }
            if s.get("envelope").get("payload").get("kind").as_str() == Some("decision") {
                cx.fail(format!("{w}: a decision envelope is not a continuation"));
            }
        }
    }
    if aseen != approval {
        cx.fail(format!(
            "approval cases missing/extra: {}",
            py_list(&sym_diff(&aseen, &approval))
        ));
    }
    let approval_case = |p: &PathBuf| {
        let n = stem(p);
        format!("approval-A{}-{}", &n[..2], &n[3..])
    };
    let negctl_path = root.join("conformance/negative-controls.json");
    let nc = if negctl_path.exists() {
        Some(OJson::read(&negctl_path)?)
    } else {
        cx.fail("conformance/negative-controls.json missing".into());
        None
    };
    if let Some(nc) = &nc {
        let mut known: BTreeSet<String> = json_files(&fix).iter().map(|p| stem(p)).collect();
        known.extend(
            json_files(&fix.join("lifecycle"))
                .iter()
                .map(|p| format!("lifecycle/{}", stem(p))),
        );
        known.extend(INJECTION.iter().map(|(n, _)| format!("injection-{n}")));
        known.extend(approval_files.iter().map(approval_case));
        let mut variants: Vec<&str> = nc.get("variants").keys();
        variants.sort();
        if variants != ["V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8"] {
            cx.fail("negative-controls.json must define V1 to V8".into());
        }
        for (v, row) in nc.get("variants").entries() {
            let must = row.get("must_fail");
            let all_known = must
                .as_arr()
                .iter()
                .all(|m| m.as_str().is_some_and(|m| known.contains(m)));
            if !must.truthy() || !all_known {
                cx.fail(format!(
                    "negative-controls.json {v}: must_fail names unknown cases"
                ));
            }
        }
    }

    // ---- digest fixtures (every digest/*.json; a "type" is a schema-checked InputRecord or Exposure)
    let mut n_dig = 0;
    for dp in json_files(&fix.join("digest")) {
        let d = OJson::read(&dp)?;
        let dn = format!("digest/{}", stem(&dp));
        n_dig += 1;
        let got = jcs(d.get("value"));
        if d.get("expected_jcs").as_str() != Some(got.as_str()) {
            cx.fail(format!("{dn}: expected_jcs mismatch"));
        }
        let sha = format!("sha256:{}", sha_hex(got.as_bytes()));
        if d.get("expected_sha256").as_str() != Some(sha.as_str()) {
            cx.fail(format!("{dn}: sha256 mismatch"));
        }
        match d.get("type").as_str() {
            Some("InputRecord") => {
                cx.check(&v_input, d.get("value"), &dn);
            }
            Some("Exposure") => {
                let wrapped = OJson::from_value(&json!({
                    "kind": "tool_request", "request_id": "r", "tool": {"name": "t"}, "arguments": {},
                    "provenance": {"dialect": "d", "parser_version": "0.1.0", "exposure": d.get("value").to_value()}}));
                cx.check(&v_intent, &wrapped, &dn);
            }
            _ if d.has("type") => {
                cx.fail(format!("{dn}: unknown type {}", py_repr(d.get("type"))));
            }
            _ => {}
        }
    }

    // ---- InputRecord examples: one per source class (plan cut P2, protocol gate P3)
    let have_in: BTreeSet<String> = json_files(&fix.join("input"))
        .iter()
        .map(|p| stem(p))
        .collect();
    let classes: BTreeSet<String> = INPUT_CLASSES.iter().map(|(c, _)| c.to_string()).collect();
    if have_in != classes {
        cx.fail(format!(
            "input examples differ: {}",
            py_list(&sym_diff(&have_in, &classes))
        ));
    }
    for (cls, kind) in INPUT_CLASSES {
        let ip = fix.join("input").join(format!("{cls}.json"));
        if !ip.exists() {
            continue;
        }
        let rec = OJson::read(&ip)?;
        if cx.check(&v_input, &rec, &format!("input/{cls}"))
            && rec.get("content_kind").as_str() != Some(kind)
        {
            cx.fail(format!("input/{cls}: content_kind must be {kind}"));
        }
        if matches!(
            rec.get("trust").as_str(),
            Some("trusted_runtime") | Some("user_supplied")
        ) && !matches!(cls, "runtime-generated" | "user-request")
        {
            cx.fail(format!(
                "input/{cls}: only runtime-generated and user-request material may be trusted"
            ));
        }
        for k in rec.keys() {
            let bad = OJson::Obj(
                rec.entries()
                    .iter()
                    .filter(|(x, _)| x != k)
                    .cloned()
                    .collect(),
            );
            if valid(&v_input, &bad) {
                cx.fail(format!(
                    "negative control: input/{cls} without {k} validated"
                ));
            }
        }
        let mut bad = rec.clone();
        bad.set("trust", "trusted".into());
        if valid(&v_input, &bad) {
            cx.fail(format!(
                "negative control: input/{cls} with bad trust validated"
            ));
        }
    }
    let bad_floor = OJson::from_value(
        &json!({"kind": "tool_request", "request_id": "r", "tool": {"name": "t"}, "arguments": {},
        "provenance": {"dialect": "d", "parser_version": "0.1.0", "exposure": {"inputs": [], "floor": "trusted"}}}),
    );
    if valid(&v_intent, &bad_floor) {
        cx.fail("negative control: exposure with bad floor validated".into());
    }

    // ---- dialect fixtures
    let (mut n_dial, mut n_int) = (0, 0);
    let dial_root = root.join("dialects/fixtures");
    let mut dial_dirs: Vec<PathBuf> = std::fs::read_dir(&dial_root)
        .map_err(|e| format!("{}: {e}", dial_root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dial_dirs.sort();
    let mut dial_files: Vec<PathBuf> = dial_dirs.iter().flat_map(|d| json_files(d)).collect();
    dial_files.sort();
    for p in &dial_files {
        let f = OJson::read(p)?;
        let where_ = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned();
        n_dial += 1;
        for k in ["input", "model", "trace_id", "turn", "expected"] {
            if !f.has(k) {
                cx.fail(format!("{where_}: missing {k}"));
            }
        }
        let e = f.get("expected");
        for k in ["text", "intents", "rejected", "partial", "dialect_version"] {
            if !e.has(k) {
                cx.fail(format!("{where_}: expected missing {k}"));
            }
        }
        for (i, it) in e.get("intents").as_arr().iter().enumerate() {
            n_int += 1;
            cx.check(&v_intent, it, &format!("{where_} intent[{i}]"));
            if !it.get("provenance").get("source_digest").is_null() {
                cx.fail(format!("{where_}: source_digest must be null"));
            }
        }
        if let Some(first) = e.get("intents").as_arr().first() {
            let mut wrong = first.clone();
            wrong.set("kind", "result".into());
            if valid(&v_intent, &wrong) {
                cx.fail("negative control: wrong kind validated".into());
            }
        }
    }
    let req: [(&str, &[&str]); 4] = [
        (
            "openai",
            &[
                "single",
                "multiple",
                "arguments_not_json",
                "arguments_not_object",
                "missing_name",
                "reasoning_content_present",
                "plain_answer",
                "legacy_function_call",
                "arguments_decoded_object",
                "arguments_not_string",
            ],
        ),
        (
            "openai_stream",
            &[
                "assembled-arguments-not-json",
                "capture-llamacpp-no-reasoning",
                "capture-llamacpp-reasoning",
                "capture-ollama-no-reasoning",
                "capture-ollama-reasoning",
                "capture-sglang-no-reasoning",
                "capture-sglang-reasoning",
                "crlf-comments-no-space",
                "cut-after-complete-arguments",
                "cut-mid-arguments",
                "error-chunk-not-json",
                "error-conflicting-name",
                "error-event-after-done",
                "error-fragment-without-index",
                "error-input-not-text",
                "error-second-choice",
                "finish-length",
                "fragmented-call",
                "plain-text-cut",
                "plain-text-stop",
                "sglang-null-repeats",
                "two-calls-interleaved",
                "unterminated-last-event",
            ],
        ),
        (
            "qwen35",
            &[
                "valid_single_call",
                "multiple_calls",
                "malformed_unclosed_function",
                "reasoning_plus_call",
                "plain_answer",
                "unknown_tool_name",
                "partial_truncated_call",
                "multiline_parameter",
                "json_typed_parameter",
                "hermes_json",
            ],
        ),
        (
            "aien_legacy",
            &[
                "valid_single_call",
                "multiple_calls",
                "reasoning_plus_call",
                "repaired_bracket",
                "repaired_missing_braces",
                "trailing_comma_rejected",
                "single_quotes_rejected",
                "unterminated_rejected",
                "fenced_json_fallback",
                "placeholder_name_rejected",
                "missing_arguments",
                "arguments_not_object",
                "plain_answer",
            ],
        ),
    ];
    for (dia, names) in req {
        let have: BTreeSet<String> = json_files(&dial_root.join(dia))
            .iter()
            .map(|p| stem(p))
            .collect();
        let want: BTreeSet<String> = names.iter().map(|s| s.to_string()).collect();
        if have != want {
            cx.fail(format!(
                "dialect {dia} fixtures differ: {}",
                py_list(&sym_diff(&have, &want))
            ));
        }
    }

    // ---- gate I corpus accounting (bench/PROTOCOL-0.3.md 3.2): counted from fixture metadata. C10
    // (approval manipulation) is supplied by lane A: every approval fixture except A01.
    let mut corpus: Vec<(&str, usize)> = MIN.iter().map(|(c, _)| (*c, 0)).collect();
    for p in json_files(&fix.join("injection")) {
        let f = OJson::read(&p)?;
        let cat = f.get("injection").get("category").as_str().unwrap_or("");
        match corpus.iter_mut().find(|(c, _)| *c == cat) {
            Some(slot) => slot.1 += 1,
            None => return Err(format!("{}: unknown category {cat}", p.display())),
        }
    }
    let c10 = approval_files
        .iter()
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with("01-"))
        .count();
    corpus.iter_mut().find(|(c, _)| *c == "C10").unwrap().1 += c10;
    for ((c, have), (_, min)) in corpus.iter().zip(MIN) {
        if *have < min {
            cx.fail(format!("gate I: {c} has {have} cases, minimum {min}"));
        }
    }
    let total: usize = corpus.iter().map(|(_, n)| n).sum();
    if total < 52 {
        cx.fail(format!("gate I: corpus has {total} cases, minimum 52"));
    }
    // gate N (b): the union of negative-control failures touches all 13 categories
    let mut cat_of: Vec<(String, &str)> = INJECTION
        .iter()
        .map(|(n, c)| (format!("injection-{n}"), *c))
        .collect();
    cat_of.extend(
        approval_files
            .iter()
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with("01-"))
            .map(|p| (approval_case(p), "C10")),
    );
    let nc = nc.unwrap_or(OJson::Null);
    let hit: BTreeSet<String> = nc
        .get("variants")
        .entries()
        .iter()
        .flat_map(|(_, r)| r.get("must_fail").as_arr().iter())
        .filter_map(|m| m.as_str())
        .filter_map(|m| {
            cat_of
                .iter()
                .find(|(n, _)| n == m)
                .map(|(_, c)| c.to_string())
        })
        .collect();
    let all_cats: BTreeSet<String> = MIN.iter().map(|(c, _)| c.to_string()).collect();
    if hit != all_cats {
        let missing: Vec<String> = all_cats.difference(&hit).cloned().collect();
        cx.fail(format!(
            "gate N: negative controls do not touch categories {}",
            py_list(&missing)
        ));
    }
    let per_cat: Vec<String> = corpus.iter().map(|(c, n)| format!("{c}={n}")).collect();
    let lines = vec![
        format!("gate I corpus: {total} cases; {}", per_cat.join(", ")),
        format!(
            "conformance cases: {n_cases}, lifecycle cases: {n_life}, injection cases: {n_inj}, approval cases: {n_app}, explicit envelopes validated: {n_env}, dialect fixtures: {n_dial}, dialect intents validated: {n_int}, digest fixtures: {n_dig}, input examples: {}, negative controls: ok",
            have_in.len()
        ),
    ];
    Ok(Report {
        lines,
        errors: cx.errors,
    })
}

/// Python `repr` of a small dict, for the two messages that print one.
fn py_dict_repr(v: &OJson) -> String {
    let items: Vec<String> = v
        .entries()
        .iter()
        .map(|(k, x)| format!("{}: {}", py_repr_str(k), py_repr(x)))
        .collect();
    format!("{{{}}}", items.join(", "))
}
