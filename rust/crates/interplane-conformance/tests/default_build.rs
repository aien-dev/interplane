//! The default build must not contain the negative controls (bench/PROTOCOL-0.3.md section 6).
#![cfg(not(feature = "negative-controls"))]
use std::process::Command;

#[test]
fn default_build_refuses_a_variant() {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../conformance/fixtures");
    let out = Command::new(env!("CARGO_BIN_EXE_interplane-conformance"))
        .args([fixtures, "--out", "/dev/null", "--variant", "V4"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("negative-controls"));
}

#[test]
fn default_binary_carries_no_variant_code() {
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_interplane-conformance")).unwrap();
    let marker = b"NEGCTL_BUILT_IN";
    assert!(!bytes.windows(marker.len()).any(|w| w == marker));
}
