//! `interplane-provenance verify <dir> [--judge-key <file> [--policy-sha256 <hex>]]` prints one verdict line (exit 0 on
//! PASS*, 1 on FAIL). The judge key file holds the independent judge's SEC1 hex public key.
//! `interplane-provenance gen-synthetic <dir> [weak|fix-the-test|fix-the-test-native|fix-the-test-stub]` writes the synthetic full-chain fixture
//! (ledger-slice binding; `weak` writes the record_effect_receipt/1 variant).
//! `interplane-provenance seal <dir> <skeleton.json>` seals a new COMPANION.json (never overwrites).

use std::path::Path;
use std::process::ExitCode;

fn report(v: String) -> ExitCode {
    println!("{v}");
    if v.starts_with("PASS") {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("verify"), Some(dir), None) => report(interplane_provenance::verify(Path::new(dir))),
        (Some("verify"), Some(dir), Some(flag)) if flag == "--judge-key" => {
            // The judge key and the policy pin come from the operator, never from the bundle.
            let key = a
                .get(4)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| interplane_provenance::evaluation::parse_judge_key(&t).ok());
            let pin = match (a.get(5).map(String::as_str), a.get(6), a.get(7)) {
                (None, _, _) => Ok(None),
                (Some("--policy-sha256"), Some(h), None)
                    if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    Ok(Some(h.to_ascii_lowercase()))
                }
                _ => Err(()),
            };
            match (key, pin) {
                (Some(k), Ok(policy_sha256)) => report(interplane_provenance::verify_with(
                    Path::new(dir),
                    &interplane_provenance::Options {
                        judge_key: Some(k),
                        policy_sha256,
                    },
                )),
                _ => {
                    eprintln!("--judge-key needs a file holding the judge's SEC1 hex public key; --policy-sha256 needs 64 hex characters");
                    ExitCode::from(2)
                }
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
            eprintln!("usage: interplane-provenance verify <dir> [--judge-key <pubkey-hex-file> [--policy-sha256 <hex>]] | gen-synthetic <dir> [weak|fix-the-test|fix-the-test-native|fix-the-test-stub] | seal <dir> <skeleton.json>");
            ExitCode::from(2)
        }
    }
}
