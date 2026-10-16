//! The subset rule refuses a malformed table entry by name instead of reading it as empty.
use std::path::Path;

use interplane_conformance::ojson::OJson;
use interplane_conformance::subset::Corpus;

fn corpus() -> Corpus {
    Corpus::new(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

/// The shared table with the `aien` entry changed by `edit`.
fn table_with(edit: impl Fn(&mut serde_json::Value)) -> OJson {
    let mut t = corpus().load_table().unwrap().to_value();
    edit(&mut t["adapters"]["aien"]);
    OJson::parse(&t.to_string()).unwrap()
}

fn refused(table: &OJson) -> String {
    corpus()
        .plans("aien", table)
        .expect_err("a malformed entry is refused")
}

#[test]
fn the_shared_entry_is_accepted() {
    let table = corpus().load_table().unwrap();
    assert!(!corpus().plans("aien", &table).unwrap().is_empty());
}

#[test]
fn a_non_string_prefix_is_refused() {
    let t = table_with(|e| e["effects"]["write_file"]["args"]["path"]["prefix"] = 7.into());
    assert!(
        refused(&t).contains("effects.write_file.args.path.prefix must be a string"),
        "{}",
        refused(&t)
    );
}

#[test]
fn a_missing_approval_map_is_refused() {
    let t = table_with(|e| {
        e.as_object_mut().unwrap().remove("approval_effects");
    });
    assert!(
        refused(&t).contains("table entry has no approval_effects"),
        "{}",
        refused(&t)
    );
}
