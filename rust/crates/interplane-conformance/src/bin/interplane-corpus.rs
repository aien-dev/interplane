//! Corpus tools for the INTERPLANE conformance corpus.
//!
//!   interplane-corpus [--root <repo>] schemas                    check every spec/schemas/*.json against the 2020-12 meta-schema
//!   interplane-corpus [--root <repo>] validate                   validate the conformance and dialect fixtures
//!   interplane-corpus [--root <repo>] trust-digest --check       recompute and compare with conformance/TRUST-DIGEST.txt
//!   interplane-corpus [--root <repo>] trust-digest --write       rewrite conformance/TRUST-DIGEST.txt
//!   interplane-corpus [--root <repo>] subset --subset <adapter>  print the adapter's subset, one fixture per line
//!   interplane-corpus [--root <repo>] subset --plans <adapter>   print the translated plans of the subset as JSON
//!
//! Without --root the repository is the nearest directory, from the current one upward, that holds
//! conformance/adapter-translation.json. Exit 0 on success, 1 on a failed check, 2 on a usage error.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use interplane_conformance::ojson::{dumps, OJson, Style};
use interplane_conformance::subset::Corpus;
use interplane_conformance::{fixture_check, trust_digest};

const USAGE: &str = "usage: interplane-corpus [--root <repo>] schemas | validate | trust-digest --check|--write | subset --subset|--plans <adapter>";

fn find_root() -> Option<PathBuf> {
    let mut d = std::env::current_dir().ok()?;
    loop {
        if d.join("conformance/adapter-translation.json").is_file() {
            return Some(d);
        }
        if !d.pop() {
            return None;
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn fail(e: String) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let root = if args.first().map(String::as_str) == Some("--root") {
        if args.len() < 2 {
            return usage();
        }
        let r = PathBuf::from(args.remove(1));
        args.remove(0);
        r
    } else {
        match find_root() {
            Some(r) => r,
            None => {
                eprintln!("no repository found above the current directory; pass --root");
                return ExitCode::from(2);
            }
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["schemas"] => match fixture_check::self_check_schemas(&root) {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        ["validate"] => match fixture_check::run(&root) {
            Ok(r) => {
                print!("{}", r.text());
                if r.passed() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => fail(e),
        },
        ["trust-digest", mode @ ("--check" | "--write")] => trust(&root, mode),
        ["subset", mode @ ("--subset" | "--plans"), adapter] => subset(&root, mode, adapter),
        _ => usage(),
    }
}

fn trust(root: &Path, mode: &str) -> ExitCode {
    let want = match trust_digest::compute(root) {
        Ok(w) => w,
        Err(e) => return fail(e),
    };
    let out = root.join(trust_digest::OUT);
    if mode == "--write" {
        if let Err(e) = std::fs::write(&out, &want) {
            return fail(format!("{}: {e}", out.display()));
        }
        println!("wrote {}", out.display());
        return ExitCode::SUCCESS;
    }
    if std::fs::read_to_string(&out).ok().as_deref() != Some(want.as_str()) {
        println!("TRUST-DIGEST.txt is stale or missing; expected:\n{want}");
        return ExitCode::from(1);
    }
    println!("TRUST-DIGEST.txt matches");
    ExitCode::SUCCESS
}

fn subset(root: &Path, mode: &str, adapter: &str) -> ExitCode {
    let corpus = Corpus::new(root);
    let table = match corpus.load_table() {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    if !table.get("adapters").has(adapter) {
        eprintln!("{adapter} is not in conformance/adapter-translation.json");
        return usage();
    }
    if mode == "--subset" {
        match corpus.subset(adapter, &table) {
            Ok(names) => println!("{}", names.join("\n")),
            Err(e) => return fail(e),
        }
    } else {
        match corpus.plans(adapter, &table) {
            Ok(plans) => print!("{}", dumps(&OJson::Arr(plans), Style::SORTED)),
            Err(e) => return fail(e),
        }
    }
    ExitCode::SUCCESS
}
