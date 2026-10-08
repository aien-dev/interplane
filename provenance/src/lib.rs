//! INTERPLANE companion provenance manifest, schema 0 (v0), and its offline verifier.
//!
//! The manifest (`COMPANION.json`) sits beside retained, immutable records and binds them link by
//! link: corpus BOM and license assertions -> training plan and run -> exported model files ->
//! AIEN candidate and loaded model -> INTERPLANE trace -> runtime effect receipt. It reuses the
//! identities those records already carry (WALDO record digests, raw-file SHA-256, the AIEN
//! candidate manifest, the aien-cli daemon load line, INTERPLANE envelopes and JCS digests, the
//! aien-cli effect receipt). It mints no ids, decides nothing and grants nothing: provenance is
//! evidence, not authority. Specification: provenance/README.md.
//!
//! `verify(dir)` returns one verdict line. It runs the checks in a fixed order and stops at the
//! first failure, so the same archive always yields the same bytes:
//! `PASS complete`, `PASS_LABELLED_INCOMPLETE missing=<sorted list>` or `FAIL <code>: <detail>`.

pub mod binding;
pub mod evaluation;
pub mod gojson;
mod ledger;
mod modelturn;
mod strict;
pub mod synth;
mod testrun;

use binding::{compact_sorted, LEDGER_BINDING, RECEIPT_BINDING, SERDE_DIGEST_FORM};
use gojson::{compact, sha256_hex, top_level_member, waldo_sha256};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub const KIND: &str = "interplane-provenance-companion";
pub const SCHEMA: u64 = 0;
pub const COMPANION_FILE: &str = "COMPANION.json";
/// The only load-support result under which an `aien` link may be present.
pub const SUPPORTED: &str = "supported_structural";
const LINKS: [&str; 6] = [
    "lineage",
    "export",
    "load_support",
    "aien",
    "interplane",
    "effect",
];

/// A refusal: stable code plus the exact place it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail(pub String);

fn fail<T>(code: &str, detail: impl AsRef<str>) -> Result<T, Fail> {
    Err(Fail(format!("FAIL {code}: {}", detail.as_ref())))
}

/// One retained (or explicitly not retained) record.
struct Record {
    path: String,
    sha256: String,
    waldo_sha256: Option<String>,
    bytes: Option<Vec<u8>>,
}

struct Archive {
    records: BTreeMap<String, Record>,
}

impl Archive {
    fn has(&self, name: &str) -> bool {
        self.records.contains_key(name)
    }
    /// Bytes of a retained record; None when the record is labelled not retained.
    fn bytes(&self, name: &str) -> Result<Option<&[u8]>, Fail> {
        match self.records.get(name) {
            None => fail("missing_record_entry", name),
            Some(r) => Ok(r.bytes.as_deref()),
        }
    }
    fn json(&self, name: &str) -> Result<Option<Value>, Fail> {
        match self.bytes(name)? {
            None => Ok(None),
            Some(b) => match strict::parse(b) {
                Some(v) => Ok(Some(v)),
                None => fail("unparseable_record", name),
            },
        }
    }
    fn sha(&self, name: &str) -> Result<&str, Fail> {
        match self.records.get(name) {
            None => fail("missing_record_entry", name),
            Some(r) => Ok(&r.sha256),
        }
    }
    /// WALDO record digest: computed from retained bytes, else the declared `waldo_sha256`.
    fn waldo(&self, name: &str) -> Result<Option<String>, Fail> {
        match self.records.get(name) {
            None => fail("missing_record_entry", name),
            Some(r) => Ok(match &r.bytes {
                Some(b) => Some(waldo_sha256(b)),
                None => r.waldo_sha256.clone(),
            }),
        }
    }
}

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    cur.as_str()
}

fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path.is_relative()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

/// True when any component of `p` under `dir` (the record file included) is a symlink. A component
/// that does not exist is left to the read (`missing_record`). A link could point anywhere.
fn crosses_symlink(dir: &Path, p: &str) -> bool {
    let mut cur = dir.to_path_buf();
    Path::new(p).components().any(|c| {
        cur.push(c);
        std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink())
    })
}

/// What the operator supplies out of band. Nothing in here is ever read from the bundle.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The independent judge's P-256 public key. With it, a bundle must carry a valid
    /// `rsi-eval/2` evaluation; without it, an evaluation in the bundle fails `evaluation_untrusted`.
    pub judge_key: Option<p256::ecdsa::VerifyingKey>,
    /// SHA-256 (hex) of the operator's own copy of the evaluation policy. With it, the retained
    /// policy must be exactly that file (`evaluation_policy_mismatch` otherwise).
    pub policy_sha256: Option<String>,
}

/// Verify the archive rooted at `dir`. Never panics on hostile input; always one line.
pub fn verify(dir: &Path) -> String {
    verify_with(dir, &Options::default())
}

pub fn verify_with(dir: &Path, opts: &Options) -> String {
    match verify_inner(dir, opts) {
        Ok((missing, effect, no_candidate)) => {
            let complete = missing.is_empty();
            let mut line = if complete {
                "PASS complete".to_string()
            } else {
                format!(
                    "PASS_LABELLED_INCOMPLETE missing={}",
                    missing.into_iter().collect::<Vec<_>>().join(",")
                )
            };
            if let Some(e) = effect {
                line.push_str(&format!(" effect={e}"));
            }
            // A complete chain run without a frozen candidate manifest says so on the line itself.
            if complete && no_candidate {
                line.push_str(" candidate=none");
            }
            line
        }
        Err(Fail(line)) => line,
    }
}

/// What the effect link proved, named by binding and strength.
/// `aien-ledger-slice/1:strong` binds request, trace, approval and bytes through records the
/// daemon wrote; `record_effect_receipt/1:weak` binds tool, outcome and digests only.
type EffectLabel = String;

/// Missing items, the effect label, and whether the AIEN link names no frozen candidate.
fn verify_inner(
    dir: &Path,
    opts: &Options,
) -> Result<(BTreeSet<String>, Option<EffectLabel>, bool), Fail> {
    let raw = match std::fs::read(dir.join(COMPANION_FILE)) {
        Ok(b) => b,
        Err(_) => return fail("missing_companion", COMPANION_FILE),
    };
    let m: Value = match strict::parse(&raw) {
        Some(v) => v,
        None => return fail("unparseable_companion", COMPANION_FILE),
    };
    if s(&m, &["kind"]) != Some(KIND) || m.get("schema").and_then(Value::as_u64) != Some(SCHEMA) {
        return fail("unsupported_companion", "kind/schema");
    }

    // 1. Records: retained bytes match their size and digest, or the record is labelled.
    let mut archive = Archive {
        records: BTreeMap::new(),
    };
    let mut actually_missing = BTreeSet::new();
    let records = match m.get("records").and_then(Value::as_object) {
        Some(r) => r,
        None => return fail("malformed_companion", "records"),
    };
    for (name, r) in records {
        let retained = match r.get("retained") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return fail("malformed_companion", format!("records.{name}.retained")),
        };
        let sha = match s(r, &["sha256"]) {
            Some(x) if x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit()) => x.to_string(),
            _ => return fail("malformed_companion", format!("records.{name}.sha256")),
        };
        let declared_waldo = s(r, &["waldo_sha256"]).map(str::to_string);
        let bytes = if retained {
            let path = match s(r, &["path"]) {
                Some(p) if safe_relative(p) => p,
                _ => return fail("malformed_companion", format!("records.{name}.path")),
            };
            if crosses_symlink(dir, path) {
                return fail(
                    "malformed_companion",
                    format!("records.{name}.path (symlink)"),
                );
            }
            let b = match std::fs::read(dir.join(path)) {
                Ok(b) => b,
                Err(_) => return fail("missing_record", name),
            };
            let want = r.get("bytes").and_then(Value::as_u64);
            if want != Some(b.len() as u64) {
                return fail(
                    "size_mismatch",
                    format!("{name} declared={} actual={}", want.unwrap_or(0), b.len()),
                );
            }
            if sha256_hex(&b) != sha {
                return fail("digest_mismatch", name);
            }
            if let Some(w) = &declared_waldo {
                if &waldo_sha256(&b) != w {
                    return fail("digest_mismatch", format!("{name}.waldo_sha256"));
                }
            }
            Some(b)
        } else {
            actually_missing.insert(format!("record:{name}"));
            None
        };
        archive.records.insert(
            name.clone(),
            Record {
                path: s(r, &["path"]).unwrap_or("").to_string(),
                sha256: sha,
                waldo_sha256: declared_waldo,
                bytes,
            },
        );
    }

    // 2. Completeness: every link is an object or an explicit null; the label matches reality.
    for link in LINKS {
        match m.get(link) {
            Some(Value::Object(_)) => {}
            Some(Value::Null) => {
                actually_missing.insert(format!("link:{link}"));
            }
            _ => {
                return fail(
                    "malformed_companion",
                    format!("{link} must be an object or null"),
                )
            }
        }
    }
    for item in modelturn::implied_missing(&archive, &m) {
        actually_missing.insert(item);
    }
    actually_missing.extend(testrun::implied_missing(&m));
    let state = s(&m, &["completeness", "state"]);
    let declared: BTreeSet<String> = m
        .get("completeness")
        .and_then(|c| c.get("missing"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    match state {
        Some("complete") if actually_missing.is_empty() && declared.is_empty() => {}
        Some("complete") => {
            let first = actually_missing.iter().next().cloned().unwrap_or_default();
            return fail("unlabelled_missing", format!("{first} (declared complete)"));
        }
        Some("incomplete") if declared == actually_missing && !declared.is_empty() => {}
        Some("incomplete") => {
            let diff: Vec<_> = declared
                .symmetric_difference(&actually_missing)
                .cloned()
                .collect();
            return fail("completeness_mismatch", diff.join(","));
        }
        _ => return fail("malformed_companion", "completeness.state"),
    }
    let link = |name: &str| m.get(name).filter(|v| v.is_object());

    // 3. Lineage: corpus BOM and licenses -> plan -> run -> managed model BOM.
    let mut run_id: Option<String> = None;
    let mut model_id: Option<String> = None;
    if let Some(l) = link("lineage") {
        match s(l, &["kind"]) {
            Some("waldo_run") => {
                let (r, mi) = check_waldo_lineage(&archive, l)?;
                run_id = Some(r);
                model_id = Some(mi);
            }
            Some("unknown_pretraining") => {
                if let Some(n) = archive.records.keys().find(|n| n.starts_with("waldo_")) {
                    return fail(
                        "lineage_conflict",
                        format!("unknown_pretraining with record {n}"),
                    );
                }
                if s(l, &["origin", "model_id"]).is_none()
                    || s(l, &["origin", "revision"]).is_none()
                {
                    return fail("malformed_companion", "lineage.origin.model_id/revision");
                }
            }
            _ => return fail("malformed_companion", "lineage.kind"),
        }
    }

    // 4. Export: the release BOM (a conversion is its own record) pins every exported file.
    if let Some(e) = link("export") {
        check_export(&archive, e, run_id.as_deref(), model_id.as_deref())?;
    }

    // 5. Load support: a check result, never a promise.
    let computed = match (link("export"), link("load_support")) {
        (Some(_), Some(ls)) => {
            let c = compute_load_support(&archive)?;
            let d = s(ls, &["declared"]).unwrap_or("");
            if let Some(c) = &c {
                if c != d {
                    return fail(
                        "load_support_mismatch",
                        format!("computed={c} declared={d}"),
                    );
                }
            }
            c
        }
        (None, Some(_)) => return fail("malformed_companion", "load_support without export"),
        _ => None,
    };

    // 6. AIEN: candidate manifest and the daemon's load line name the exported bytes.
    if let Some(a) = link("aien") {
        if link("export").is_none() {
            return fail("malformed_companion", "aien without export");
        }
        if let Some(c) = &computed {
            if c != SUPPORTED {
                return fail("load_unsupported", c);
            }
        }
        check_aien(&archive, a)?;
    }

    // 7 and 8. INTERPLANE trace, then the effect for exactly that request.
    let mut call: Option<Call> = None;
    if let Some(t) = link("interplane") {
        if link("aien").is_none() {
            return fail("malformed_companion", "interplane without aien");
        }
        call = check_trace(&archive, t)?;
    }
    let mut effect_label = None;
    let mut ledger_checked = false;
    if let Some(e) = link("effect") {
        let (Some(t), Some(aien)) = (link("interplane"), link("aien")) else {
            return fail("malformed_companion", "effect without interplane");
        };
        match s(e, &["binding"]) {
            Some(LEDGER_BINDING) => {
                // Who authored the proposal must be stated, never implied by silence. A scripted
                // turn leaves `link:model_turn` missing; a model generation is checked in
                // `modelturn` against the daemon's own generation record.
                let origin = match s(e, &["proposal_origin"]) {
                    Some(modelturn::SCRIPTED) => modelturn::SCRIPTED,
                    Some(modelturn::GENERATED) => modelturn::GENERATED,
                    Some(modelturn::SUPERSEDED) => {
                        return fail(
                            "unsupported_binding",
                            "effect.proposal_origin=model_generation/1 (superseded by model_generation/2: needs the daemon's own record)",
                        )
                    }
                    Some(other) => {
                        return fail(
                            "unsupported_binding",
                            format!("effect.proposal_origin={other}"),
                        )
                    }
                    None => return fail("malformed_companion", "effect.proposal_origin"),
                };
                ledger::check(
                    &archive,
                    e,
                    t,
                    aien,
                    call.as_ref(),
                    testrun::claims_test_run(&archive, &m),
                )?;
                if origin == modelturn::GENERATED {
                    modelturn::check(&archive, t, call.as_ref())?;
                }
                effect_label = Some(format!("{LEDGER_BINDING}:strong proposal={origin}"));
                ledger_checked = true;
            }
            Some(RECEIPT_BINDING) => {
                // Weaker: tool + digests only. The label stays on the verdict.
                if s(e, &["digest_form"]) != Some(SERDE_DIGEST_FORM) {
                    return fail(
                        "canonicalization_mismatch",
                        format!(
                            "effect.digest_form={} (producer form is {SERDE_DIGEST_FORM})",
                            s(e, &["digest_form"]).unwrap_or("absent")
                        ),
                    );
                }
                if let Some(c) = &call {
                    check_receipt(&archive, t, &c.args, &c.result)?;
                }
                effect_label = Some(format!("{RECEIPT_BINDING}:weak"));
            }
            Some(other) => return fail("unsupported_binding", format!("effect.binding={other}")),
            None => return fail("malformed_companion", "effect.binding"),
        }
    }
    testrun::check(&archive, &m, ledger_checked)?;
    evaluation::check(&archive, &m, ledger_checked, opts)?;
    // Test material may verify, but never as a complete chain: `fixture.class` other than `real`
    // refuses `complete` whatever records it carries (#76 review). Checked last, so every deeper
    // check above still runs on test material.
    if let Some(f) = m.get("fixture") {
        match s(f, &["class"]) {
            Some("real") => {}
            Some(c) if actually_missing.is_empty() => {
                return fail("synthetic_complete", format!("fixture.class={c}"))
            }
            Some(_) => {}
            None => return fail("malformed_companion", "fixture.class"),
        }
    }
    let no_candidate = link("aien").is_some_and(|a| s(a, &["candidate_id"]).is_none());
    Ok((actually_missing, effect_label, no_candidate))
}

fn check_waldo_lineage(a: &Archive, l: &Value) -> Result<(String, String), Fail> {
    let run_id = s(l, &["run_id"]).unwrap_or("").to_string();
    let Some(run_bom_bytes) = a.bytes("waldo_run_bom")? else {
        return fail(
            "malformed_companion",
            "waldo_run_bom must be retained (it carries the corpus BOM)",
        );
    };
    let run_bom: Value = a.json("waldo_run_bom")?.unwrap_or(Value::Null);
    if s(&run_bom, &["kind"]) != Some("openwaldo-bom")
        || s(&run_bom, &["subject"]) != Some("training-run")
        || s(&run_bom, &["id"]) != Some(run_id.as_str())
    {
        return fail("binding_mismatch", "run_bom.kind/subject/id");
    }
    // Corpus BOM embedded in the run BOM: its WALDO digest is the run's corpus pin.
    let c = compact(run_bom_bytes);
    let corpus = match top_level_member(&c, "corpus_bom") {
        Some(x) => x,
        None => return fail("binding_mismatch", "run_bom.corpus_bom absent"),
    };
    if Some(sha256_hex(corpus).as_str()) != s(&run_bom, &["corpus_bom_sha256"]) {
        return fail("binding_mismatch", "run_bom.corpus_bom_sha256");
    }
    let cb = &run_bom["corpus_bom"];
    if s(cb, &["kind"]) != Some("openwaldo-bom") || s(cb, &["subject"]) != Some("corpus") {
        return fail("binding_mismatch", "corpus_bom.kind/subject");
    }
    let licenses: Vec<Value> = cb
        .get("licenses")
        .and_then(Value::as_object)
        .map(|o| o.keys().map(|k| Value::String(k.clone())).collect())
        .unwrap_or_default();
    if l.get("corpus_license_assertions") != Some(&Value::Array(licenses)) {
        return fail("binding_mismatch", "lineage.corpus_license_assertions");
    }
    if l.get("training_backend") != run_bom.get("execution").and_then(|e| e.get("backend")) {
        return fail("binding_mismatch", "lineage.training_backend");
    }
    let model_id = s(&run_bom, &["model_id"]).unwrap_or("").to_string();
    if s(l, &["model_id"]) != Some(model_id.as_str()) {
        return fail("binding_mismatch", "lineage.model_id");
    }
    // Preflight is pinned by raw bytes inside the run BOM.
    if Some(a.sha("waldo_preflight")?) != s(&run_bom, &["preflight", "sha256"]) {
        return fail("binding_mismatch", "run_bom.preflight.sha256");
    }
    // Run record: same run BOM, terminal state complete.
    if let Some(run) = a.json("waldo_run")? {
        let rb = waldo_sha256(run_bom_bytes);
        if s(&run, &["bom_sha256"]) != Some(rb.as_str())
            || s(&run, &["id"]) != Some(run_id.as_str())
        {
            return fail("binding_mismatch", "run.bom_sha256");
        }
        if s(&run, &["state"]) != Some("complete") {
            return fail("run_not_complete", s(&run, &["state"]).unwrap_or("absent"));
        }
    }
    // Plan: its WALDO digest is the model identity.
    if let Some(p) = a.waldo("waldo_plan")? {
        if p != model_id {
            return fail("binding_mismatch", "plan_sha256 != run_bom.model_id");
        }
    }
    // Managed model BOM: pins the run BOM, selects the run, pins the run's weights.
    if let Some(mb) = a.json("waldo_model_bom")? {
        if s(&mb, &["model_id"]) != Some(model_id.as_str())
            || s(&mb, &["plan_sha256"]) != Some(model_id.as_str())
        {
            return fail("binding_mismatch", "model_bom.model_id/plan_sha256");
        }
        if s(&mb, &["current_run_id"]) != Some(run_id.as_str()) {
            return fail("binding_mismatch", "model_bom.current_run_id");
        }
        let pin = mb
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|r| r.iter().find(|p| s(p, &["id"]) == Some(run_id.as_str())));
        let Some(pin) = pin else {
            return fail(
                "binding_mismatch",
                format!("model_bom.runs[{run_id}] absent"),
            );
        };
        if s(pin, &["bom_sha256"]) != Some(waldo_sha256(run_bom_bytes).as_str()) {
            return fail(
                "binding_mismatch",
                format!("model_bom.runs[{run_id}].bom_sha256"),
            );
        }
        if pin.get("simulated") != Some(&Value::Bool(false))
            || s(pin, &["state"]) != Some("complete")
        {
            return fail("run_not_complete", format!("model_bom.runs[{run_id}]"));
        }
        let w = pin
            .get("artifacts")
            .and_then(Value::as_array)
            .and_then(|x| x.iter().find(|y| s(y, &["role"]) == Some("weights")));
        if w.and_then(|w| s(w, &["sha256"])) != Some(a.sha("run_weights")?) {
            return fail(
                "binding_mismatch",
                format!("model_bom.runs[{run_id}].artifacts[weights]"),
            );
        }
    }
    Ok((run_id, model_id))
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn check_export(
    a: &Archive,
    e: &Value,
    run_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), Fail> {
    for n in ["export_weights", "export_config", "export_tokenizer"] {
        a.sha(n)?;
    }
    match (a.has("export_chat_template"), e.get("chat_template")) {
        (true, Some(Value::String(r))) if r == "export_chat_template" => {}
        (false, Some(Value::Null)) => {}
        _ => {
            return fail(
                "malformed_companion",
                "export.chat_template must name export_chat_template or be null",
            )
        }
    }
    let Some(run_id) = run_id else {
        // External lineage: no WALDO release record exists; files are pinned by this manifest
        // and, when present, by the AIEN candidate manifest.
        if a.has("release_bom") {
            return fail("lineage_conflict", "release_bom without waldo_run lineage");
        }
        return Ok(());
    };
    let Some(rel) = a.json("release_bom")? else {
        return fail(
            "malformed_companion",
            "release_bom must be retained for waldo_run lineage",
        );
    };
    if s(&rel, &["kind"]) != Some("openwaldo-bom") || s(&rel, &["subject"]) != Some("model-release")
    {
        return fail("binding_mismatch", "release_bom.kind/subject");
    }
    if s(&rel, &["format"]) != s(e, &["format"]) {
        return fail("binding_mismatch", "release_bom.format");
    }
    if s(&rel, &["run_id"]) != Some(run_id) || s(&rel, &["model_id"]) != model_id {
        return fail("binding_mismatch", "release_bom.run_id/model_id");
    }
    if a.waldo("waldo_model_bom")?.as_deref() != s(&rel, &["source_bom_sha256"]) {
        return fail("binding_mismatch", "release_bom.source_bom_sha256");
    }
    let arts = rel
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for n in [
        "export_weights",
        "export_config",
        "export_tokenizer",
        "export_chat_template",
    ] {
        if !a.has(n) {
            continue;
        }
        let name = file_name(&a.records[n].path);
        let hit = arts.iter().find(|x| s(x, &["path"]) == Some(name));
        if hit.and_then(|h| s(h, &["sha256"])) != Some(a.sha(n)?) {
            return fail("binding_mismatch", format!("release_bom.artifacts[{name}]"));
        }
    }
    // Conversion check: WALDO's Hugging Face export rewrites the safetensors header and copies
    // the tensor data section byte for byte (docs/MODEL-EXPORTS.md).
    if s(e, &["format"]) == Some("huggingface") {
        if let (Some(rw), Some(ew)) = (a.bytes("run_weights")?, a.bytes("export_weights")?) {
            if safetensors_data(rw).is_none() || safetensors_data(rw) != safetensors_data(ew) {
                return fail(
                    "conversion_mismatch",
                    "safetensors data section run_weights != export_weights",
                );
            }
        }
    }
    Ok(())
}

fn safetensors_data(b: &[u8]) -> Option<&[u8]> {
    let n = u64::from_le_bytes(b.get(..8)?.try_into().ok()?) as usize;
    b.get(8usize.checked_add(n)?..)
}

/// Structural load-support check against what aien-cli loads today: a `LlamaForCausalLM`
/// config.json and a Hugging Face `tokenizer.json` (aien-inference-abi `Tokenizer::from_file`,
/// the `tokenizers` crate). Returns None when the needed bytes are not retained.
fn compute_load_support(a: &Archive) -> Result<Option<String>, Fail> {
    let (Some(cfg), Some(tok)) = (a.json("export_config")?, a.bytes("export_tokenizer")?) else {
        return Ok(None);
    };
    let arch = cfg
        .get("architectures")
        .and_then(Value::as_array)
        .and_then(|x| x.first())
        .and_then(Value::as_str)
        .unwrap_or("absent");
    if arch != "LlamaForCausalLM" {
        return Ok(Some(format!("unsupported_architecture:{arch}")));
    }
    let tj: Value = strict::parse(tok).unwrap_or(Value::Null);
    let vocab = tj
        .get("model")
        .and_then(|m| m.get("vocab"))
        .and_then(Value::as_object);
    let Some(vocab) = vocab else {
        let class = s(&tj, &["tokenizer_class"]).unwrap_or("not_tokenizer_json");
        return Ok(Some(format!("unsupported_tokenizer:{class}")));
    };
    let mut ids: BTreeSet<u64> = vocab.values().filter_map(Value::as_u64).collect();
    if let Some(added) = tj.get("added_tokens").and_then(Value::as_array) {
        ids.extend(
            added
                .iter()
                .filter_map(|t| t.get("id").and_then(Value::as_u64)),
        );
    }
    let tok_n = ids.iter().next_back().map(|m| m + 1).unwrap_or(0);
    let cfg_n = cfg.get("vocab_size").and_then(Value::as_u64).unwrap_or(0);
    if tok_n != cfg_n {
        return fail(
            "incompatible",
            format!("config.vocab_size={cfg_n} tokenizer_vocab={tok_n}"),
        );
    }
    Ok(Some(SUPPORTED.to_string()))
}

/// Minimal reader for the CandidateManifestV1 subset used here: `[section]` and `key = "value"`.
/// None when a key repeats in a section (no last-wins reading).
fn toml_table(text: &str) -> Option<BTreeMap<(String, String), String>> {
    let mut out = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            if let Some(rest) = v.strip_prefix('"') {
                if let Some(end) = rest.find('"') {
                    let key = (section.clone(), k.trim().to_string());
                    if out.insert(key, rest[..end].to_string()).is_some() {
                        return None;
                    }
                }
            }
        }
    }
    Some(out)
}

fn check_aien(a: &Archive, l: &Value) -> Result<(), Fail> {
    // Optional record: no entry at all means no AIEN candidate manifest exists (not a missing record).
    let cand = if a.has("candidate_manifest") {
        a.bytes("candidate_manifest")?
    } else {
        None
    };
    if cand.is_none() && s(l, &["candidate_id"]).is_some() {
        // A candidate claim must be backed by its manifest; absence is only honest with candidate_id null.
        return fail(
            "binding_mismatch",
            "candidate_id without candidate_manifest",
        );
    }
    if let Some(b) = cand {
        let Some(t) = toml_table(&String::from_utf8_lossy(b)) else {
            return fail("binding_mismatch", "candidate manifest repeats a key");
        };
        let get = |sec: &str, k: &str| t.get(&(sec.to_string(), k.to_string())).map(String::as_str);
        if get("", "schema") != Some("CandidateManifestV1") {
            return fail("binding_mismatch", "candidate.schema");
        }
        if get("", "id") != s(l, &["candidate_id"]) {
            return fail("binding_mismatch", "candidate.id");
        }
        let exe = s(l, &["executable"]).unwrap_or("");
        if get("executables", exe).is_none()
            || get("executables", exe) != s(l, &["executable_sha256"])
        {
            return fail("binding_mismatch", format!("candidate.executables.{exe}"));
        }
        for (key, rec) in [
            ("model-safetensors-sha256", "export_weights"),
            ("tokenizer-json-sha256", "export_tokenizer"),
            ("config-json-sha256", "export_config"),
        ] {
            if get("model", key) != Some(a.sha(rec)?) {
                return fail("binding_mismatch", format!("candidate.model.{key}"));
            }
        }
    }
    if let Some(b) = a.bytes("aien_load_log")? {
        let text = String::from_utf8_lossy(b);
        let Some(line) = text
            .lines()
            .rev()
            .find(|x| x.contains("checkpoint loaded from "))
        else {
            return fail("binding_mismatch", "aien_load.line absent");
        };
        let field = |k: &str| {
            line.split([' ', ',', ')'])
                .find_map(|w| w.strip_prefix(&format!("{k}=")))
                .unwrap_or("")
                .to_string()
        };
        if !line.contains("(tokenizer loaded,") {
            return fail("binding_mismatch", "aien_load.tokenizer missing");
        }
        if field("model_sha256") != a.sha("export_weights")? {
            return fail("binding_mismatch", "aien_load.model_sha256");
        }
        if field("tokenizer_sha256") != a.sha("export_tokenizer")? {
            return fail("binding_mismatch", "aien_load.tokenizer_sha256");
        }
    }
    Ok(())
}

/// The retained request, as the trace holds it.
pub(crate) struct Call {
    pub tool: String,
    pub args: Value,
    pub result: Value,
    /// The `tool_request` payload and the envelope's `source`, as the trace holds them.
    pub req_payload: Value,
    pub req_source: Value,
}

/// Returns the request's tool, arguments and result payload, when the trace bytes are retained.
fn check_trace(a: &Archive, t: &Value) -> Result<Option<Call>, Fail> {
    let Some(trace) = a.json("interplane_trace")? else {
        return Ok(None);
    };
    let (Some(tid), Some(rid)) = (s(t, &["trace_id"]), s(t, &["request_id"])) else {
        return fail("malformed_companion", "interplane.trace_id/request_id");
    };
    let Some(envs) = trace.as_array() else {
        return fail("unparseable_record", "interplane_trace");
    };
    let (mut req, mut result, mut final_seen) = (None, None, false);
    for (i, e) in envs.iter().enumerate() {
        if let Err(code) = interplane_core::validate_envelope_value(e) {
            return fail(
                "invalid_envelope",
                format!("interplane_trace[{i}] {code:?}"),
            );
        }
        if s(e, &["trace_id"]) != Some(tid) {
            return fail(
                "binding_mismatch",
                format!("interplane_trace[{i}].trace_id"),
            );
        }
        let p = &e["payload"];
        if s(p, &["request_id"]) == Some(rid) {
            match s(p, &["kind"]) {
                Some("tool_request") if req.is_some() => {
                    return fail(
                        "binding_mismatch",
                        format!("interplane_trace[{i}] second tool_request for {rid}"),
                    )
                }
                Some("tool_request") => {
                    req = Some((
                        s(p, &["tool", "name"]).unwrap_or("").to_string(),
                        p.get("arguments").cloned().unwrap_or(Value::Null),
                        p.clone(),
                        e.get("source").cloned().unwrap_or(Value::Null),
                    ))
                }
                Some("result") => {
                    // spec/CORE.md lifecycle: at most one `requires_approval`, then at most one final
                    // result, and nothing after the final one.
                    let pending = s(p, &["status"]) == Some("requires_approval");
                    if final_seen || (pending && result.is_some()) {
                        return fail(
                            "binding_mismatch",
                            format!("interplane_trace[{i}] result after the final one for {rid}"),
                        );
                    }
                    final_seen = !pending;
                    result = Some(p.clone());
                }
                _ => {}
            }
        }
    }
    match (req, result) {
        (Some((tool, args, req_payload, req_source)), Some(result)) => Ok(Some(Call {
            tool,
            args,
            result,
            req_payload,
            req_source,
        })),
        _ => fail("not_in_trace", format!("{tid}/{rid}")),
    }
}

/// SHA-256 of JCS (RFC 8785) bytes. Used only to recognise a digest made in the wrong form.
fn jcs_digest(v: &Value) -> String {
    format!(
        "sha256:{}",
        sha256_hex(interplane_core::canonicalize(v).as_bytes())
    )
}

/// The digest aien-cli computes: SHA-256 of `serde_json::to_vec` (form
/// `serde_json.to_vec.sorted-keys/1`, see binding.rs), refused when not reproducible.
fn producer_digest(what: &str, v: &Value) -> Result<String, Fail> {
    match compact_sorted(v) {
        Ok(b) => Ok(format!("sha256:{}", sha256_hex(&b))),
        Err(why) => fail(
            "canonicalization_mismatch",
            format!("{what}: producer form {SERDE_DIGEST_FORM} not reproduced: {why}"),
        ),
    }
}

/// Weaker binding. The aien-cli effect receipt (crates/aien-cli/src/tools.rs
/// `record_effect_receipt`, version 1) carries no trace or request id; it is bound to the request
/// only through its tool, outcome and the digests of the request's arguments and result data.
/// Two requests with identical tool, arguments and result data are not told apart.
fn check_receipt(a: &Archive, t: &Value, args: &Value, result: &Value) -> Result<(), Fail> {
    let Some(r) = a.json("effect_receipt")? else {
        return Ok(());
    };
    let rid = s(t, &["request_id"]).unwrap_or("");
    if r.get("version").and_then(Value::as_u64) != Some(1) {
        return fail("binding_mismatch", "effect_receipt.version");
    }
    if s(&r, &["tool"]) != s(result, &["provenance", "capability"]) {
        return fail(
            "binding_mismatch",
            format!("effect_receipt.tool request={rid}"),
        );
    }
    for (field, value) in [
        ("arguments_digest", args),
        ("result_digest", &result["data"]),
    ] {
        let what = format!("effect_receipt.{field}");
        let want = producer_digest(&what, value)?;
        if s(&r, &[field]) != Some(want.as_str()) {
            if s(&r, &[field]) == Some(jcs_digest(value).as_str()) {
                return fail(
                    "canonicalization_mismatch",
                    format!("{what} is the JCS form; the producer form is {SERDE_DIGEST_FORM}"),
                );
            }
            return fail("binding_mismatch", format!("{what} request={rid}"));
        }
    }
    let ok = s(result, &["status"]) == Some("ok");
    if r.get("success") != Some(&Value::Bool(ok)) {
        return fail(
            "binding_mismatch",
            format!("effect_receipt.success request={rid}"),
        );
    }
    Ok(())
}
