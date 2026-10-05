//! Negative controls (bench/PROTOCOL-0.3.md section 6): only with `--features negative-controls`.
#![cfg(feature = "negative-controls")]
use std::path::PathBuf;

use interplane_conformance::negctl::{matrix, Variant};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance")
}

#[test]
fn unmodified_runner_passes_everything() {
    let m = matrix(
        &root().join("fixtures"),
        &root().join("negative-controls.json"),
    )
    .unwrap();
    assert_eq!(m["baseline_failed"], serde_json::json!([]));
}

#[test]
fn every_variant_turns_a_pass_into_a_detected_failure() {
    let m = matrix(
        &root().join("fixtures"),
        &root().join("negative-controls.json"),
    )
    .unwrap();
    assert_eq!(m["valid"], serde_json::json!(true), "{m}");
    for v in Variant::ALL {
        let row = &m["variants"][v.name()];
        assert_eq!(
            row["detected"],
            serde_json::json!(true),
            "{}: {row}",
            v.name()
        );
        assert!(
            !row["failed"].as_array().unwrap().is_empty(),
            "{}",
            v.name()
        );
    }
}

#[test]
fn variant_names_round_trip() {
    for v in Variant::ALL {
        assert_eq!(Variant::parse(v.name()), Some(v));
    }
    assert_eq!(Variant::parse("V9"), None);
}
