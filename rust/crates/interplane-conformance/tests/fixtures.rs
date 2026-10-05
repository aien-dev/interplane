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

#[test]
fn expanded_in_capability_is_still_denied_and_a_changed_selection_is_detected() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/fixtures");
    let all = load_fixtures(&dir).unwrap();
    assert_eq!(all.len(), 27);
    let mut fx = all
        .into_iter()
        .find(|f| f["case"] == "24-expansion-requested-excluded")
        .unwrap();
    let run = run_case(&fx);
    assert_eq!(run.selections.len(), 1);
    let names: Vec<&str> = run.selections[0]["selected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["send_email", "write_file"]);
    assert_eq!(run.observed.len(), 1);
    assert_eq!(run.observed[0].status.as_str(), "denied");
    assert_eq!((run.decide_calls, run.execute_calls), (1, 0));
    // negative control: a wrong expected selection must be reported.
    fx["expected"]["selections"][0]["selection_digest"] = serde_json::json!("sha256:00");
    assert!(!compare(&fx, &run).is_empty());
}
