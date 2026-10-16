//! The shared adapter runner reproduces the two recorded adapter runs: the independent to-do
//! reproduction and the maintainer's smart-home check, each judged normally and with
//! `--no-exposure-check` (the negative control, which must fail).
//!
//! The smart-home files are matched byte for byte. The recorded to-do host never set its runtime
//! id (the Python SDK then left `provenance.runtime` null in every pipeline-made result), which the
//! Rust `RuntimeAuthority` trait cannot express. Its recorded files are therefore matched with
//! `result_digest` masked, and byte for byte against `tests/hosts/toy-verdicts*.json`: the same
//! host and runner run by the retired Python code with `runtime_id = "toy"` set.
mod hosts;

use std::path::{Path, PathBuf};

use interplane_conformance::adapter_runner::{run, AdapterHost, Verdicts};
use interplane_conformance::ojson::OJson;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hosts")
}

/// `{"<name>": <entry>}` as the runner's `--entry` file holds it.
fn entry(path: &Path) -> (String, OJson) {
    let given = OJson::read(path).unwrap();
    let [(name, e)] = given.entries() else {
        panic!("{}: one entry only", path.display())
    };
    (name.clone(), e.clone())
}

/// Equal text, or the produced file is written next to the build for a diff and the test fails.
fn same(v: &Verdicts, want: &str, label: &str, mask: bool) {
    let strip = |t: &str| -> String {
        t.lines()
            .filter(|l| !(mask && l.trim_start().starts_with("\"result_digest\": ")))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let got = v.file_text();
    if strip(&got) != strip(want) || (!mask && got != want) {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
        std::fs::write(&out, &got).unwrap();
        panic!("{label}: verdicts differ; got {}", out.display());
    }
}

struct Case<'a> {
    host: &'a dyn AdapterHost,
    entry: PathBuf,
    recorded: &'a str,
    /// Byte-exact reference files, when the recorded ones can only be matched with digests masked.
    exact: Option<[&'a str; 2]>,
    /// (summary key, value) for the normal run and for the negative control.
    expect: [(&'a str, i128); 2],
}

fn check(c: Case) {
    let root = root();
    let (name, e) = entry(&c.entry);
    for (i, file) in ["verdicts.json", "verdicts-negctl.json"]
        .into_iter()
        .enumerate()
    {
        let negctl = i == 1;
        let v = run(&root, &name, Some(&e), c.host, negctl).unwrap();
        let recorded = std::fs::read_to_string(root.join(c.recorded).join(file)).unwrap();
        same(&v, &recorded, &format!("{name}-{file}"), c.exact.is_some());
        if let Some(exact) = c.exact {
            let want = std::fs::read_to_string(here().join(exact[i])).unwrap();
            same(&v, &want, &format!("{name}-exact-{file}"), false);
        }
        assert_eq!(v.passed(), !negctl, "{name} {file}: pass/fail");
        let (key, n) = c.expect[i];
        assert!(
            matches!(v.summary().get(key), OJson::Int(x) if *x == n),
            "{name} {file}: {key}"
        );
    }
}

#[test]
fn toy_reproduction_matches_recorded_verdicts() {
    check(Case {
        host: &hosts::Toy,
        entry: here().join("toy-entry.json"),
        recorded: "bench/runs/adapter-repro-20261005T1215Z/toy",
        exact: Some(["toy-verdicts.json", "toy-verdicts-negctl.json"]),
        expect: [("executed", 38), ("failed", 18)],
    });
}

#[test]
fn home_check_matches_recorded_verdicts() {
    check(Case {
        host: &hosts::Home,
        entry: root().join("bench/runs/home-check-20261005T1320Z/table-entry.json"),
        recorded: "bench/runs/home-check-20261005T1320Z",
        exact: None,
        expect: [("executed", 43), ("failed", 30)],
    });
}
