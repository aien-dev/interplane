use std::process::ExitCode;

use interplane_probe::*;

fn usage() -> ExitCode {
    eprintln!("usage: interplane-probe --endpoint URL --model M [--out report.json] [--skip NAME[,NAME]]... [--backend NAME] [--model-revision REV] [--timeout SECS]");
    eprintln!("credentials: set INTERPLANE_PROBE_API_KEY (never printed, never written)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut cfg = ProbeConfig {
        environment: detect_environment(),
        ..Default::default()
    };
    let (mut out, mut timeout) = (None, 120u64);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next();
        match a.as_str() {
            "--endpoint" => cfg.endpoint = val().unwrap_or_default(),
            "--model" => cfg.model = val().unwrap_or_default(),
            "--out" => out = val(),
            "--skip" => cfg.skip.extend(
                val()
                    .unwrap_or_default()
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            ),
            "--backend" => cfg.backend = val(),
            "--model-revision" => cfg.environment.model_revision = val(),
            "--timeout" => timeout = val().and_then(|s| s.parse().ok()).unwrap_or(120),
            _ => return usage(),
        }
    }
    if cfg.endpoint.is_empty() || cfg.model.is_empty() {
        return usage();
    }
    let key = std::env::var("INTERPLANE_PROBE_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let transport = UreqTransport::new(&cfg.endpoint, key, timeout);
    let report = run_probes(&transport, &cfg);
    println!("endpoint {}  model {}", report.endpoint, report.model);
    for p in &report.probes {
        println!(
            "{:<24} {:<12} {}",
            p.name,
            p.verdict.as_str(),
            p.detail.as_deref().unwrap_or("")
        );
    }
    for p in &report.profiles {
        println!("{:<28} {}", p.name, p.status.as_str());
    }
    if let Some(path) = out {
        if let Err(e) = std::fs::write(&path, report_text(&report) + "\n") {
            eprintln!("cannot write {path}: {e}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}
