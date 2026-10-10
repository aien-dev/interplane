//! Standalone scanner: `replay_scan <dir-or-file>...` scans every `.json` file below each path,
//! recursively, and exits non-zero if any file has a finding. Run it on anything you plan to
//! commit as a replay fixture (including drafts outside `dialects/replay`).
use interplane_lenshift::replay::scan;
use serde_json::Value;
use std::path::Path;

fn walk(p: &Path, bad: &mut usize, n: &mut usize) {
    if p.is_dir() {
        let mut es: Vec<_> = std::fs::read_dir(p)
            .expect("read dir")
            .map(|e| e.unwrap().path())
            .collect();
        es.sort();
        for e in es {
            walk(&e, bad, n);
        }
    } else if p.extension().and_then(|e| e.to_str()) == Some("json") {
        *n += 1;
        let t = std::fs::read_to_string(p).expect("read");
        let found = match serde_json::from_str::<Value>(&t) {
            Ok(v) => scan(&v),
            Err(e) => vec![format!("not JSON: {e}")],
        };
        if !found.is_empty() {
            *bad += 1;
            eprintln!("{}:\n  {}", p.display(), found.join("\n  "));
        }
    }
}

fn main() {
    let (mut bad, mut n) = (0, 0);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: replay_scan <dir-or-file>...");
        std::process::exit(2);
    }
    for a in &args {
        walk(Path::new(a), &mut bad, &mut n);
    }
    println!("scanned {n} files, {bad} with findings");
    std::process::exit(if bad == 0 && n > 0 { 0 } else { 1 });
}
