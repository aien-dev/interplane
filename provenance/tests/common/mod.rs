//! Helpers shared by the verdict test files. Mutations happen on copies under
//! CARGO_TARGET_TMPDIR; committed fixtures are never rewritten.
#![allow(dead_code)]

use interplane_provenance::gojson::{sha256_hex, waldo_sha256};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn copy_dir(from: &Path, to: &Path) {
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

pub fn scratch(fixture: &str, name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verdicts")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture), &d);
    d
}

pub fn companion(d: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(d.join("COMPANION.json")).unwrap()).unwrap()
}

pub fn write_companion(d: &Path, v: &Value) {
    std::fs::write(
        d.join("COMPANION.json"),
        serde_json::to_vec_pretty(v).unwrap(),
    )
    .unwrap();
}

pub fn path_of(d: &Path, rec: &str) -> PathBuf {
    d.join(companion(d)["records"][rec]["path"].as_str().unwrap())
}

/// What an attacker (or a careless tool) does after changing a record: update its entry in the
/// manifest so the file-level digest matches. The chain must still refuse.
pub fn restamp(d: &Path, rec: &str) {
    let b = std::fs::read(path_of(d, rec)).unwrap();
    let mut c = companion(d);
    c["records"][rec]["sha256"] = json!(sha256_hex(&b));
    c["records"][rec]["bytes"] = json!(b.len());
    if c["records"][rec].get("waldo_sha256").is_some() {
        c["records"][rec]["waldo_sha256"] = json!(waldo_sha256(&b));
    }
    write_companion(d, &c);
}
