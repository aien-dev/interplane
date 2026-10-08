//! `interplane-provenance verify <dir>` prints one verdict line (exit 0 on PASS*, 1 on FAIL).
//! `interplane-provenance gen-synthetic <dir> [weak|fix-the-test|fix-the-test-native|fix-the-test-stub]` writes the synthetic full-chain fixture
//! (ledger-slice binding; `weak` writes the record_effect_receipt/1 variant).
//! `interplane-provenance seal <dir> <skeleton.json>` seals a new COMPANION.json (never overwrites).

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("verify"), Some(dir), None) => {
            let v = interplane_provenance::verify(Path::new(dir));
            println!("{v}");
            if v.starts_with("PASS") {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        (Some("gen-synthetic"), Some(dir), mode @ (None | Some(_))) => {
            let opts = interplane_provenance::synth::Opts {
                weak: mode.map(String::as_str) == Some("weak"),
                fix_the_test: matches!(
                    mode.map(String::as_str),
                    Some("fix-the-test" | "fix-the-test-native" | "fix-the-test-stub")
                ),
                native: match mode.map(String::as_str) {
                    Some("fix-the-test-native") => Some(true),
                    Some("fix-the-test-stub") => Some(false),
                    _ => None,
                },
                ..Default::default()
            };
            match interplane_provenance::synth::write_synthetic(Path::new(dir), &opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(2)
                }
            }
        }
        (Some("seal"), Some(dir), Some(sk)) => {
            let sk = std::fs::read(sk)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            match sk.map(|v| interplane_provenance::synth::seal(Path::new(dir), v)) {
                Some(Ok(())) => ExitCode::SUCCESS,
                Some(Err(e)) => {
                    eprintln!("{e}");
                    ExitCode::from(2)
                }
                None => {
                    eprintln!("unreadable skeleton");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("usage: interplane-provenance verify <dir> | gen-synthetic <dir> [weak|fix-the-test|fix-the-test-native|fix-the-test-stub] | seal <dir> <skeleton.json>");
            ExitCode::from(2)
        }
    }
}
