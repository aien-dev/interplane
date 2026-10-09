//! Harness side of the VAC M5 slice: the RSI engine proposes a change to the fixture's README, the
//! separate judge builds parent and candidate, runs its holdouts and signs a version 2 receipt
//! (spark-rsi docs/RECEIPT-V2.md). Both run under their own operating-system accounts through the
//! operator's hook script (scripts/m5-propose-and-judge.sh); this module only reads what they
//! wrote. Nothing here grants, approves or decides anything: the gate before the write is
//! `interplane_provenance::evaluation::precheck`, and the daemon's normal approval still applies.
use crate::provenance_export::Evaluation;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// The one file the M5 task allows to change.
pub const TARGET: &str = "README.md";

/// The RSI engine's proposal and the judge's evidence for it.
#[derive(Debug, Clone)]
pub struct Judged {
    pub proposal_id: String,
    /// The whole new file the proposal writes.
    pub content: String,
    pub evaluation: Evaluation,
}

/// Reads one `ImprovementProposal` (spark-rsi `propose` output) and refuses any proposal that is
/// not a whole-file change to `allowed`. A proposal aimed at a holdout, the policy or any other
/// file stops here, before anything reaches the judge's signature check or the daemon.
pub fn check_proposal(bytes: &[u8], allowed: &str) -> Result<(String, String), String> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("proposal is not JSON: {e}"))?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("proposal has no {k}"))
    };
    let target = field("target_file")?;
    if target != allowed {
        return Err(format!(
            "RSI proposed a change to {target}; the task allows only {allowed}: refused"
        ));
    }
    Ok((
        field("id")?.to_string(),
        field("proposed_patch")?.to_string(),
    ))
}

/// The hook's switches for making counterfeit evidence in the gate's negative checks. A real run
/// never passes them on (the hook also ignores them unless `M5_GATE_NEGATIVE=1`).
pub const NEGATIVE_ONLY_ENV: [&str; 4] = [
    "M5_GATE_NEGATIVE",
    "M5_SUBJECT_OVERRIDE",
    "M5_JUDGE_KEY_OVERRIDE",
    "M5_RECEIPT_V1",
];

/// Runs `hook <ws> <out>` and reads `proposal.json`, `receipt.json` and `policy.json` from `out`,
/// which must not exist yet. The receipt and policy are kept as the exact bytes written, and the
/// policy must be byte for byte `pinned_policy`, the operator's own copy: the hook's copy is never
/// trusted on its own. The hook's output goes to `out/hook.log`.
pub fn propose_and_judge(
    hook: &Path,
    ws: &Path,
    out: &Path,
    pinned_policy: &[u8],
) -> Result<Judged, String> {
    run_hook(hook, ws, out, pinned_policy, None)
}

/// The same hook and the same checks, for a proposal a real model turn produced: `proposal` is a
/// `proposal.json`-shaped file the harness wrote from the model's tool call (`id`, `target_file`,
/// `proposed_patch`). The hook skips `spark-rsi propose`; the judge still builds and evaluates the
/// exact bytes, and the receipt and policy are checked as for any other proposal.
pub fn judge_model_proposal(
    hook: &Path,
    ws: &Path,
    out: &Path,
    pinned_policy: &[u8],
    proposal: &Path,
) -> Result<Judged, String> {
    run_hook(hook, ws, out, pinned_policy, Some(proposal))
}

fn run_hook(
    hook: &Path,
    ws: &Path,
    out: &Path,
    pinned_policy: &[u8],
    proposal: Option<&Path>,
) -> Result<Judged, String> {
    if out.exists() {
        return Err(format!(
            "{} exists: a judged run is never reused",
            out.display()
        ));
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(hook);
    cmd.arg(ws).arg(out);
    for v in NEGATIVE_ONLY_ENV {
        cmd.env_remove(v);
    }
    // Only the model-turn row may redirect the proposal; the RSI row never does.
    cmd.env_remove("M5_PROPOSAL_FILE");
    if let Some(p) = proposal {
        cmd.env("M5_PROPOSAL_FILE", p);
    }
    let run = cmd
        .output()
        .map_err(|e| format!("{}: {e}", hook.display()))?;
    let log = [run.stdout.as_slice(), run.stderr.as_slice()].concat();
    std::fs::write(out.join("hook.log"), &log).map_err(|e| e.to_string())?;
    if !run.status.success() {
        return Err(format!(
            "hook {} exited {}: {}",
            hook.display(),
            run.status,
            String::from_utf8_lossy(&log).lines().last().unwrap_or("")
        ));
    }
    let read = |n: &str| std::fs::read(out.join(n)).map_err(|e| format!("{n}: {e}"));
    let (proposal_id, content) = check_proposal(&read("proposal.json")?, TARGET)?;
    let policy = read("policy.json")?;
    if policy != pinned_policy {
        return Err("the judged policy is not the operator's pinned policy: refused".into());
    }
    Ok(Judged {
        proposal_id,
        content,
        evaluation: Evaluation {
            receipt: read("receipt.json")?,
            policy,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proposal(target: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"id": "p1", "title": "t", "description": "d",
            "target_file": target, "proposed_patch": "new text\n", "kind": "Style",
            "created_at": "2026-10-08T00:00:00Z", "sandbox_path": null}))
        .unwrap()
    }

    #[test]
    fn readme_proposal_is_accepted() {
        let (id, content) = check_proposal(&proposal("README.md"), TARGET).unwrap();
        assert_eq!((id.as_str(), content.as_str()), ("p1", "new text\n"));
    }

    #[test]
    fn proposals_aimed_at_holdouts_policy_or_code_are_refused() {
        for t in [
            "holdouts/HOLD-M5-001.json",
            "policy.json",
            "../m5/policy.json",
            "src/main.rs",
            "TASK.md",
        ] {
            let e = check_proposal(&proposal(t), TARGET).unwrap_err();
            assert!(e.contains("refused"), "{t}: {e}");
        }
        assert!(check_proposal(b"{", TARGET).is_err());
    }

    #[test]
    fn a_used_output_directory_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let e = propose_and_judge(Path::new("/bin/true"), d.path(), d.path(), b"{}").unwrap_err();
        assert!(e.contains("never reused"), "{e}");
    }
}
