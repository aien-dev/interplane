//! One test per verdict. Every expected verdict is a literal: the verifier must print exactly
//! these bytes. Mutations happen on copies under CARGO_TARGET_TMPDIR; committed fixtures are
//! never rewritten.

use interplane_provenance::gojson::{sha256_hex, waldo_sha256};
use interplane_provenance::synth::{self, Opts};
use interplane_provenance::verify;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SYN: &str = "fixtures/synthetic-full-chain";
const REAL: &str = "fixtures/real-waldo-smoke";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn scratch(fixture: &str, name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verdicts")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture), &d);
    d
}

fn companion(d: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(d.join("COMPANION.json")).unwrap()).unwrap()
}

fn write_companion(d: &Path, v: &Value) {
    std::fs::write(
        d.join("COMPANION.json"),
        serde_json::to_vec_pretty(v).unwrap(),
    )
    .unwrap();
}

fn path_of(d: &Path, rec: &str) -> PathBuf {
    d.join(companion(d)["records"][rec]["path"].as_str().unwrap())
}

/// What an attacker (or a careless tool) does after changing a record: update its entry in the
/// manifest so the file-level digest matches. The chain must still refuse.
fn restamp(d: &Path, rec: &str) {
    let b = std::fs::read(path_of(d, rec)).unwrap();
    let mut c = companion(d);
    c["records"][rec]["sha256"] = json!(sha256_hex(&b));
    c["records"][rec]["bytes"] = json!(b.len());
    if c["records"][rec].get("waldo_sha256").is_some() {
        c["records"][rec]["waldo_sha256"] = json!(waldo_sha256(&b));
    }
    write_companion(d, &c);
}

#[test]
fn valid_self_contained_archive() {
    assert_eq!(verify(Path::new(SYN)), "PASS complete");
}

#[test]
fn synthetic_fixture_is_reproducible_byte_for_byte() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("regen");
    let _ = std::fs::remove_dir_all(&d);
    synth::write_synthetic(&d, &Opts::default()).unwrap();
    let c = companion(&d);
    for (_, r) in c["records"].as_object().unwrap() {
        let p = r["path"].as_str().unwrap();
        assert_eq!(
            std::fs::read(d.join(p)).unwrap(),
            std::fs::read(Path::new(SYN).join(p)).unwrap(),
            "{p}"
        );
    }
    assert_eq!(
        std::fs::read(d.join("COMPANION.json")).unwrap(),
        std::fs::read(Path::new(SYN).join("COMPANION.json")).unwrap()
    );
}

#[test]
fn real_waldo_export_is_labelled_incomplete() {
    assert_eq!(
        verify(Path::new(REAL)),
        "PASS_LABELLED_INCOMPLETE missing=link:aien,link:effect,link:interplane"
    );
}

#[test]
fn changed_weight_bytes() {
    let d = scratch(SYN, "changed_weight_bytes");
    let p = path_of(&d, "export_weights");
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0x01;
    std::fs::write(&p, b).unwrap();
    assert_eq!(verify(&d), "FAIL digest_mismatch: export_weights");
}

#[test]
fn changed_weight_bytes_with_restamped_manifest() {
    let d = scratch(SYN, "changed_weight_bytes_restamped");
    let p = path_of(&d, "export_weights");
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0x01;
    std::fs::write(&p, b).unwrap();
    restamp(&d, "export_weights");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: release_bom.artifacts[model.safetensors]"
    );
}

#[test]
fn tokenizer_swapped_and_restamped() {
    let d = scratch(SYN, "tokenizer_swapped");
    let p = path_of(&d, "export_tokenizer");
    let mut t: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    t["model"]["vocab"]["world"] = json!(5);
    t["model"]["vocab"]["note"] = json!(4);
    std::fs::write(&p, serde_json::to_vec_pretty(&t).unwrap()).unwrap();
    restamp(&d, "export_tokenizer");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: release_bom.artifacts[tokenizer.json]"
    );
}

#[test]
fn config_vocab_disagrees_with_tokenizer() {
    // A fully consistent chain (every digest recomputed) whose config cannot serve its tokenizer.
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verdicts")
        .join("config_vocab");
    let _ = std::fs::remove_dir_all(&d);
    synth::write_synthetic(
        &d,
        &Opts {
            config_vocab: Some(9),
        },
    )
    .unwrap();
    assert_eq!(
        verify(&d),
        "FAIL incompatible: config.vocab_size=9 tokenizer_vocab=8"
    );
}

#[test]
fn missing_bom_in_archive_declared_complete() {
    let d = scratch(SYN, "missing_bom_unlabelled");
    std::fs::remove_file(path_of(&d, "waldo_model_bom")).unwrap();
    assert_eq!(verify(&d), "FAIL missing_record: waldo_model_bom");
}

#[test]
fn missing_bom_correctly_labelled_incomplete() {
    // The derived-export situation: only the hash of the managed model BOM survives.
    let d = scratch(SYN, "missing_bom_labelled");
    std::fs::remove_file(path_of(&d, "waldo_model_bom")).unwrap();
    let mut c = companion(&d);
    c["records"]["waldo_model_bom"]["retained"] = json!(false);
    c["completeness"] = json!({"state": "incomplete", "missing": ["record:waldo_model_bom"]});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "PASS_LABELLED_INCOMPLETE missing=record:waldo_model_bom"
    );
}

#[test]
fn missing_bom_labelled_record_but_archive_claims_complete() {
    let d = scratch(SYN, "missing_bom_mislabelled");
    std::fs::remove_file(path_of(&d, "waldo_model_bom")).unwrap();
    let mut c = companion(&d);
    c["records"]["waldo_model_bom"]["retained"] = json!(false);
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL unlabelled_missing: record:waldo_model_bom (declared complete)"
    );
}

#[test]
fn missing_bom_labelled_with_wrong_hash() {
    let d = scratch(SYN, "missing_bom_wrong_hash");
    std::fs::remove_file(path_of(&d, "waldo_model_bom")).unwrap();
    let mut c = companion(&d);
    c["records"]["waldo_model_bom"]["retained"] = json!(false);
    c["records"]["waldo_model_bom"]["waldo_sha256"] = json!("0".repeat(64));
    c["completeness"] = json!({"state": "incomplete", "missing": ["record:waldo_model_bom"]});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: release_bom.source_bom_sha256"
    );
}

#[test]
fn changed_license_metadata_with_restamped_manifest() {
    let d = scratch(SYN, "changed_license");
    let p = path_of(&d, "waldo_run_bom");
    let text = std::fs::read_to_string(&p)
        .unwrap()
        .replace("\"CC0-1.0\"", "\"MIT\"");
    std::fs::write(&p, text).unwrap();
    restamp(&d, "waldo_run_bom");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: run_bom.corpus_bom_sha256"
    );
}

#[test]
fn changed_manifest_metadata() {
    let d = scratch(SYN, "changed_manifest_metadata");
    let mut c = companion(&d);
    c["aien"]["candidate_id"] = json!("CAND-3");
    write_companion(&d, &c);
    assert_eq!(verify(&d), "FAIL binding_mismatch: candidate.id");
}

#[test]
fn truncated_record_write() {
    let d = scratch(SYN, "truncated_record");
    let p = path_of(&d, "export_weights");
    let b = std::fs::read(&p).unwrap();
    std::fs::write(&p, &b[..b.len() / 2]).unwrap();
    assert_eq!(
        verify(&d),
        "FAIL size_mismatch: export_weights declared=160 actual=80"
    );
}

#[test]
fn truncated_manifest_write() {
    let d = scratch(SYN, "truncated_manifest");
    let b = std::fs::read(d.join("COMPANION.json")).unwrap();
    std::fs::write(d.join("COMPANION.json"), &b[..b.len() / 2]).unwrap();
    assert_eq!(verify(&d), "FAIL unparseable_companion: COMPANION.json");
}

#[test]
fn receipt_for_another_request_substituted() {
    let d = scratch(SYN, "receipt_other_request");
    let other = synth::receipt_bytes(
        "write_file",
        &synth::request_args(synth::OTHER_REQUEST_ID),
        &synth::result_data(synth::OTHER_REQUEST_ID),
        true,
    );
    std::fs::write(path_of(&d, "effect_receipt"), other).unwrap();
    restamp(&d, "effect_receipt");
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: effect_receipt.arguments_digest request=call-prov-01"
    );
}

#[test]
fn receipt_claimed_for_another_trace() {
    let d = scratch(SYN, "receipt_other_trace");
    let mut c = companion(&d);
    c["interplane"]["trace_id"] = json!("trace-prov-99");
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL binding_mismatch: interplane_trace[0].trace_id"
    );
}

#[test]
fn load_claim_on_export_aien_cannot_load() {
    let d = scratch(REAL, "real_load_claim");
    let mut c = companion(&d);
    c["aien"] = json!({"candidate_id": "CAND-3", "executable": "aien-cli-native-release",
                       "executable_sha256": "3a17ee79a238ad223fd59f4436d98906a0144b8d5ac47809da194a9ac1d493db"});
    c["completeness"]["missing"] = json!(["link:effect", "link:interplane"]);
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL load_unsupported: unsupported_tokenizer:OpenWALDOByteTokenizer"
    );
}

#[test]
fn unknown_lineage_cannot_carry_training_records() {
    let d = scratch(REAL, "real_lineage_conflict");
    let mut c = companion(&d);
    c["lineage"] =
        json!({"kind": "unknown_pretraining", "origin": {"model_id": "x", "revision": "unknown"}});
    write_companion(&d, &c);
    assert_eq!(
        verify(&d),
        "FAIL lineage_conflict: unknown_pretraining with record waldo_model_bom"
    );
}

#[test]
fn sealed_manifest_is_never_rewritten() {
    let d = scratch(SYN, "no_rewrite");
    let before = std::fs::read(d.join("COMPANION.json")).unwrap();
    assert!(synth::seal(&d, companion(&d)).is_err());
    assert_eq!(std::fs::read(d.join("COMPANION.json")).unwrap(), before);
}

#[test]
fn export_weights_not_derived_from_trained_run() {
    // Weights changed, and both the manifest and the release BOM re-pinned to them: only the
    // conversion check (tensor data section equals the run's pinned weights) still refuses.
    let d = scratch(SYN, "conversion_mismatch");
    let p = path_of(&d, "export_weights");
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0x01;
    std::fs::write(&p, &b).unwrap();
    restamp(&d, "export_weights");
    let rp = path_of(&d, "release_bom");
    let mut rel: Value = serde_json::from_slice(&std::fs::read(&rp).unwrap()).unwrap();
    for a in rel["artifacts"].as_array_mut().unwrap() {
        if a["path"] == "model.safetensors" {
            a["sha256"] = json!(sha256_hex(&b));
        }
    }
    std::fs::write(&rp, serde_json::to_vec_pretty(&rel).unwrap()).unwrap();
    restamp(&d, "release_bom");
    assert_eq!(
        verify(&d),
        "FAIL conversion_mismatch: safetensors data section run_weights != export_weights"
    );
}
