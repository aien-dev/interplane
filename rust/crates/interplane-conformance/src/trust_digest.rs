//! Freeze file for the 0.3 trust corpus (bench/PROTOCOL-0.3.md section 8):
//! `conformance/TRUST-DIGEST.txt`.
//!
//! `corpus_digest` is the sha256 of the `sha256sum` lines (`<hex>  <path>\n`, repo-relative paths)
//! of every `conformance/fixtures/**/*.json` in byte order of the path (the whole corpus the
//! negative-control matrix depends on, not only the injection cases). `negative_controls_sha256`
//! freezes `conformance/negative-controls.json`, `protocol_sha256` freezes
//! `bench/PROTOCOL-0.3.md` and `adapter_translation_sha256` freezes
//! `conformance/adapter-translation.json` (the data of the subset rule). `t3_subset` and
//! `t4_subset` list the fixtures each adapter must run, computed by [`crate::subset`].
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::subset::Corpus;

pub const OUT: &str = "conformance/TRUST-DIGEST.txt";
const CORPUS: &str = "conformance/fixtures";
const NEGCTL: &str = "conformance/negative-controls.json";
const PROTOCOL: &str = "bench/PROTOCOL-0.3.md";
const TRANSLATION: &str = "conformance/adapter-translation.json";

fn sha(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

fn read(root: &Path, rel: &str) -> Result<Vec<u8>, String> {
    std::fs::read(root.join(rel)).map_err(|e| format!("{rel}: {e}"))
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        let sub = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            walk(&e.path(), &sub, out)?;
        } else if name.ends_with(".json") {
            out.push(sub);
        }
    }
    Ok(())
}

/// The text `conformance/TRUST-DIGEST.txt` must hold for the checkout at `root`.
pub fn compute(root: &Path) -> Result<String, String> {
    let mut names = vec![];
    walk(&root.join(CORPUS), "", &mut names)?;
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut lines = String::new();
    for n in &names {
        let body = read(root, &format!("{CORPUS}/{n}"))?;
        lines.push_str(&format!("{}  {CORPUS}/{n}\n", sha(&body)));
    }
    let corpus = Corpus::new(root);
    let table = corpus.load_table()?;
    let t3 = corpus.subset("odysseus", &table)?;
    let t4 = corpus.subset("aien", &table)?;
    Ok(format!(
        "corpus: {CORPUS}\ncorpus_files: {}\ncorpus_digest: sha256:{}\nnegative_controls: {NEGCTL}\nnegative_controls_sha256: sha256:{}\n\
         protocol: {PROTOCOL}\nprotocol_sha256: sha256:{}\nadapter_translation: {TRANSLATION}\nadapter_translation_sha256: sha256:{}\n\
         t3_subset_count: {}\nt3_subset: {}\nt4_subset_count: {}\nt4_subset: {}\n",
        names.len(),
        sha(lines.as_bytes()),
        sha(&read(root, NEGCTL)?),
        sha(&read(root, PROTOCOL)?),
        sha(&read(root, TRANSLATION)?),
        t3.len(),
        t3.join(","),
        t4.len(),
        t4.join(","),
    ))
}
