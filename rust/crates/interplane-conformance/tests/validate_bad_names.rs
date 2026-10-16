//! The fixture validator reports malformed corpus entries as errors and never panics: approval
//! files with a short or non-ASCII name, and an oversized case (15) with no tool call.
#![cfg(feature = "schema-check")]

use std::fs;
use std::path::{Path, PathBuf};

use interplane_conformance::fixture_check;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// A scratch copy of everything the validator reads.
fn corpus(label: &str) -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
    let _ = fs::remove_dir_all(&root);
    for d in ["spec/schemas", "conformance", "dialects/fixtures"] {
        copy_dir(&repo.join(d), &root.join(d));
    }
    root
}

fn errors_of(root: &Path) -> String {
    let report = fixture_check::run(root).expect("validator returns a report");
    assert!(!report.passed(), "a broken corpus must fail");
    report.text()
}

#[test]
fn unmodified_copy_passes() {
    let root = corpus("validate-clean");
    assert!(fixture_check::run(&root).unwrap().passed());
}

#[test]
fn short_and_non_ascii_approval_names_are_errors() {
    let root = corpus("validate-bad-names");
    let app = root.join("conformance/fixtures/approval");
    let src = app.join("01-authorized-continuation.json");
    fs::copy(&src, app.join("7.json")).unwrap();
    fs::copy(&src, app.join("\u{e9}\u{e9}-accent.json")).unwrap();
    let text = errors_of(&root);
    assert!(text.contains("approval/7: case slug"), "{text}");
    assert!(
        text.contains("approval/\u{e9}\u{e9}-accent: case slug"),
        "{text}"
    );
}

#[test]
fn oversized_case_without_a_tool_call_is_an_error() {
    let root = corpus("validate-no-call");
    let fix = root.join("conformance/fixtures");
    let p = fs::read_dir(&fix)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("15"))
        .unwrap();
    let mut f: serde_json::Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
    f["steps"] = serde_json::json!([]);
    fs::write(&p, serde_json::to_string_pretty(&f).unwrap()).unwrap();
    let text = errors_of(&root);
    assert!(
        text.contains("15: first step must carry a tool call"),
        "{text}"
    );
}
