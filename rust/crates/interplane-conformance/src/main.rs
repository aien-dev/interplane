use std::path::PathBuf;
use std::process::ExitCode;

use interplane_conformance::*;
use interplane_core::canonicalize;

const USAGE: &str =
    "usage: interplane-conformance <fixtures_dir> --out <verdicts.json> [--dump <dir>]";

fn main() -> ExitCode {
    let mut dir: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut dump: Option<PathBuf> = None;
    let mut variant: Option<String> = None;
    let mut matrix_out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--dump" => dump = args.next().map(PathBuf::from),
            "--variant" => variant = args.next(),
            "--matrix" => matrix_out = args.next().map(PathBuf::from),
            _ if dir.is_none() => dir = Some(PathBuf::from(a)),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(dir) = dir else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if variant.is_some() || matrix_out.is_some() {
        return negative_controls(&dir, out, variant, matrix_out);
    }
    run(&dir, out, dump, Ctl::default())
}

#[cfg(not(feature = "negative-controls"))]
fn negative_controls(
    _: &std::path::Path,
    _: Option<PathBuf>,
    _: Option<String>,
    _: Option<PathBuf>,
) -> ExitCode {
    eprintln!("--variant and --matrix need a build with the test-only `negative-controls` feature");
    ExitCode::from(2)
}

#[cfg(feature = "negative-controls")]
fn negative_controls(
    dir: &std::path::Path,
    out: Option<PathBuf>,
    variant: Option<String>,
    matrix_out: Option<PathBuf>,
) -> ExitCode {
    use interplane_conformance::negctl::{matrix, Variant};
    if let Some(m) = matrix_out {
        let spec = dir.join("../negative-controls.json");
        return match matrix(dir, &spec) {
            Ok(v) => {
                if let Err(e) = std::fs::write(&m, canonicalize(&v) + "\n") {
                    eprintln!("cannot write {}: {e}", m.display());
                    return ExitCode::from(2);
                }
                println!("negative-control matrix valid: {}", v["valid"]);
                if v["valid"] == serde_json::json!(true) {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        };
    }
    let Some(v) = variant.as_deref().and_then(Variant::parse) else {
        eprintln!("unknown variant (V1 to V6)");
        return ExitCode::from(2);
    };
    println!(
        "[{}] NEGATIVE CONTROL {}: failures below are expected",
        interplane_conformance::negctl::MARKER,
        v.name()
    );
    run(dir, out, None, Some(v))
}

fn run(dir: &std::path::Path, out: Option<PathBuf>, dump: Option<PathBuf>, ctl: Ctl) -> ExitCode {
    let suite = match run_suite(dir, ctl) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    println!("{:<4} {:<34} RESULT", "#", "CASE");
    for o in &suite.outcomes {
        println!(
            "{:<4} {:<34} {}",
            o.tag,
            o.name,
            if o.errs.is_empty() { "PASS" } else { "FAIL" }
        );
        for e in &o.errs {
            println!("       - {e}");
        }
        if let (Some(d), Some(results)) = (&dump, &o.results) {
            let _ = std::fs::create_dir_all(d);
            let path = d.join(format!("{}.results.json", o.name));
            if let Err(e) = std::fs::write(
                &path,
                canonicalize(&serde_json::Value::Array(results.clone())) + "\n",
            ) {
                eprintln!("cannot write {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
    }
    if let Some(o) = out {
        if let Err(e) = std::fs::write(&o, canonicalize(&verdicts(suite.rows.clone())) + "\n") {
            eprintln!("cannot write {}: {e}", o.display());
            return ExitCode::from(2);
        }
    }
    let failed = suite.failed().len();
    println!("{} of {} cases failed", failed, suite.case_count());
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
