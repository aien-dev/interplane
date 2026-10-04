use std::path::PathBuf;

use interplane_conformance::*;

#[test]
fn every_fixture_passes() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/fixtures");
    let fixtures = load_fixtures(&dir).unwrap();
    assert!(fixtures.len() >= 18);
    for fx in &fixtures {
        let run = run_case(fx);
        let errs = compare(fx, &run);
        assert!(errs.is_empty(), "{}: {errs:?}", run.case);
    }
    assert_eq!(check_digest_fixture(&dir), None);
}

#[test]
fn comparison_detects_disagreement() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/fixtures");
    let mut fx = load_fixtures(&dir).unwrap().remove(0);
    fx["expected"]["runtime"]["execute_calls"] = serde_json::json!(7);
    assert!(!compare(&fx, &run_case(&fx)).is_empty());
}

#[test]
fn verdicts_are_deterministic() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/fixtures");
    let fx = &load_fixtures(&dir).unwrap()[0];
    let a = verdict(&run_case(fx), &[]);
    let b = verdict(&run_case(fx), &[]);
    assert_eq!(
        interplane_core::canonicalize(&a),
        interplane_core::canonicalize(&b),
        "digests are stable across runs"
    );
}
