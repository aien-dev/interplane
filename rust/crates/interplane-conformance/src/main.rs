use std::path::PathBuf;
use std::process::ExitCode;

use interplane_conformance::*;
use interplane_core::canonicalize;

fn main() -> ExitCode {
    let mut dir: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut dump: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--dump" => dump = args.next().map(PathBuf::from),
            _ if dir.is_none() => dir = Some(PathBuf::from(a)),
            _ => {
                eprintln!("usage: interplane-conformance <fixtures_dir> --out <verdicts.json> [--dump <dir>]");
                return ExitCode::from(2);
            }
        }
    }
    let Some(dir) = dir else {
        eprintln!(
            "usage: interplane-conformance <fixtures_dir> --out <verdicts.json> [--dump <dir>]"
        );
        return ExitCode::from(2);
    };
    let fixtures = match load_fixtures(&dir) {
        Ok(f) if !f.is_empty() => f,
        Ok(_) => {
            eprintln!("no NN-*.json fixtures in {}", dir.display());
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut rows = vec![];
    let mut failed = 0;
    println!("{:<4} {:<34} RESULT", "#", "CASE");
    for fx in &fixtures {
        let run = run_case(fx);
        let errs = compare(fx, &run);
        println!(
            "{:<4} {:<34} {}",
            &run.case[..run.case.len().min(2)],
            run.case,
            if errs.is_empty() { "PASS" } else { "FAIL" }
        );
        for e in &errs {
            println!("       - {e}");
        }
        failed += usize::from(!errs.is_empty());
        if let Some(d) = &dump {
            let _ = std::fs::create_dir_all(d);
            let path = d.join(format!("{}.results.json", run.case));
            if let Err(e) = std::fs::write(
                &path,
                canonicalize(&serde_json::Value::Array(run.results.clone())) + "\n",
            ) {
                eprintln!("cannot write {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
        rows.push((run.case.clone(), verdict(&run, &errs)));
    }
    let lifecycle = match load_lifecycle_fixtures(&dir) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    for fx in &lifecycle {
        let case = fx["case"]
            .as_str()
            .unwrap_or("lifecycle/unnamed")
            .to_string();
        let (steps, errs) = run_lifecycle_case(fx);
        println!(
            "{:<4} {:<34} {}",
            "L",
            case,
            if errs.is_empty() { "PASS" } else { "FAIL" }
        );
        for e in &errs {
            println!("       - {e}");
        }
        failed += usize::from(!errs.is_empty());
        rows.push((case, lifecycle_verdict(&steps, &errs)));
    }
    for (name, err) in check_digest_fixtures(&dir) {
        match err {
            Some(e) => {
                println!("{:<4} {:<34} FAIL\n       - {e}", "-", name);
                failed += 1;
            }
            None => println!("{:<4} {:<34} PASS", "-", name),
        }
    }
    if let Some(o) = out {
        if let Err(e) = std::fs::write(&o, canonicalize(&verdicts(rows)) + "\n") {
            eprintln!("cannot write {}: {e}", o.display());
            return ExitCode::from(2);
        }
    }
    println!(
        "{} of {} cases failed",
        failed,
        fixtures.len() + lifecycle.len()
    );
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
