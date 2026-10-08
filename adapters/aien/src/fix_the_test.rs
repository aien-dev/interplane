//! Harness side of the fix-the-test slice (VAC M3b): prepare a task workspace from the fixture,
//! run the task's test command, and build the retained records the provenance verifier checks
//! (`vac-task`, `vac-source-pin`, `vac-test-run/1`). Specification: provenance/BINDING.md.
//!
//! These records are HARNESS evidence. The AIEN daemon only writes the file; it does not run the
//! tests, so a test run is not a daemon effect and the records are unsigned exports. Nothing here
//! grants, approves or decides anything.
use crate::provenance_export::sha256_hex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The file the scripted proposal replaces.
pub const TARGET: &str = "src/clamp.c";

/// A task workspace under git, before the fix.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub ws: PathBuf,
    pub task_id: String,
    pub argv: Vec<String>,
    /// Commit of the buggy tree.
    pub commit: String,
    /// SHA-256 of the buggy `src/clamp.c` (what the grant's `prior_sha256` must equal).
    pub target_blob_sha256: String,
    /// The corrected whole file the scripted proposal writes.
    pub solution: String,
}

/// What one run of the test command did.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub started_unix_ms: u128,
    pub duration_ms: u128,
}

fn git(ws: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(ws)
        .args([
            "-c",
            "user.name=vac-harness",
            "-c",
            "user.email=vac-harness@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn copy_tree(from: &Path, to: &Path, skip: &[&str]) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if skip.contains(&name.as_str()) {
            continue;
        }
        let dst = to.join(&name);
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&e.path(), &dst, &[])?;
        } else {
            std::fs::copy(e.path(), &dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// `task_id: x`, `test_command: a b c` and `target_file: p` lines of `TASK.md`. `target_file` is the
/// one file the task allows the agent to change; it goes into the task record as `target_path`.
fn parse_task(md: &str) -> Result<(String, Vec<String>, String), String> {
    let field = |k: &str| {
        md.lines()
            .find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()))
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("TASK.md has no {k}"))
    };
    let argv: Vec<String> = field("test_command:")?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    Ok((field("task_id:")?, argv, field("target_file:")?))
}

/// Copy `fixture` (without `solution/` and `build/`) into `ws`, `git init` and commit it.
pub fn prepare(fixture: &Path, ws: &Path) -> Result<Prepared, String> {
    let (task_id, argv, target) = parse_task(
        &std::fs::read_to_string(fixture.join("TASK.md")).map_err(|e| format!("TASK.md: {e}"))?,
    )?;
    // The scripted proposal writes TARGET. A task that allows a different file is refused here,
    // before any proposal reaches the daemon.
    if target != TARGET {
        return Err(format!(
            "TASK.md target_file {target} is not the scripted proposal target {TARGET}: refused"
        ));
    }
    let solution = std::fs::read_to_string(fixture.join("solution/clamp.c"))
        .map_err(|e| format!("solution/clamp.c: {e}"))?;
    copy_tree(fixture, ws, &["solution", "build"])?;
    git(ws, &["init", "-q"])?;
    git(ws, &["add", "-A"])?;
    git(ws, &["commit", "-q", "-m", "task start: failing test"])?;
    let commit = git(ws, &["rev-parse", "HEAD"])?;
    let buggy = std::fs::read(ws.join(TARGET)).map_err(|e| e.to_string())?;
    Ok(Prepared {
        ws: ws.to_path_buf(),
        task_id,
        argv,
        commit,
        target_blob_sha256: sha256_hex(&buggy),
        solution,
    })
}

/// Run the task's test command in the workspace and keep exactly what it printed.
pub fn run_tests(p: &Prepared) -> Result<TestOutcome, String> {
    let started = std::time::SystemTime::now();
    let t0 = std::time::Instant::now();
    let out = Command::new(&p.argv[0])
        .args(&p.argv[1..])
        .current_dir(&p.ws)
        .output()
        .map_err(|e| format!("{:?}: {e}", p.argv))?;
    Ok(TestOutcome {
        exit_code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: out.stderr,
        started_unix_ms: started
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        duration_ms: t0.elapsed().as_millis(),
    })
}

/// The records of one finished fix-the-test run.
#[derive(Debug, Clone)]
pub struct TestRunEvidence {
    pub task: Value,
    pub source_pin: Value,
    pub test_run: Value,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub task_id: String,
    /// The test exit codes the harness saw.
    pub exit_before: i32,
    pub exit_after: i32,
    pub tree_commit_after: String,
}

/// Commit the fixed tree and build the three records. `before` ran before the proposal, `after`
/// after the daemon's ack; `blob_after` is the digest of the file as the test saw it.
pub fn evidence(
    p: &Prepared,
    before: &TestOutcome,
    after: &TestOutcome,
) -> Result<TestRunEvidence, String> {
    git(&p.ws, &["add", "-A"])?;
    git(
        &p.ws,
        &[
            "commit",
            "-q",
            "-m",
            "scripted fix applied through the daemon ledger",
        ],
    )?;
    let tree_commit_after = git(&p.ws, &["rev-parse", "HEAD"])?;
    let blob_after = sha256_hex(&std::fs::read(p.ws.join(TARGET)).map_err(|e| e.to_string())?);
    let task = json!({"kind": "vac-task", "task_id": p.task_id, "repo_commit": p.commit,
        "test_cmd": p.argv, "target_path": TARGET});
    let source_pin = json!({"kind": "vac-source-pin", "repo": "fix_the_test", "commit": p.commit,
        "target_path": TARGET, "target_blob_sha256": p.target_blob_sha256});
    let test_run = json!({"kind": "vac-test-run", "v": 1, "task_id": p.task_id, "argv": p.argv,
        "cwd_rel": ".", "exit_code": after.exit_code, "test_exit_before": before.exit_code,
        "stdout_sha256": sha256_hex(&after.stdout), "stderr_sha256": sha256_hex(&after.stderr),
        "started_unix_ms": after.started_unix_ms as u64, "duration_ms": after.duration_ms as u64,
        "tree_commit_after": tree_commit_after, "target_blob_sha256_after": blob_after,
        "before": {"stdout_sha256": sha256_hex(&before.stdout), "stderr_sha256": sha256_hex(&before.stderr)}});
    Ok(TestRunEvidence {
        task,
        source_pin,
        test_run,
        stdout: after.stdout.clone(),
        stderr: after.stderr.clone(),
        task_id: p.task_id.clone(),
        exit_before: before.exit_code,
        exit_after: after.exit_code,
        tree_commit_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/fix_the_test")
    }

    #[test]
    fn task_fields_parse() {
        let (id, argv, target) =
            parse_task("# T\n\ntask_id: a/b-1\ntest_command: make test\ntarget_file: src/x.c\n")
                .unwrap();
        assert_eq!(
            (id.as_str(), argv, target.as_str()),
            ("a/b-1", vec!["make".into(), "test".into()], "src/x.c")
        );
        assert!(parse_task("task_id: x\n").is_err());
        // No target_file: the task does not say which file may change, so it is refused.
        assert!(parse_task("task_id: x\ntest_command: make test\n").is_err());
    }

    #[test]
    fn fixture_fails_before_and_passes_after_the_solution() {
        let d = tempfile::tempdir().unwrap();
        let p = prepare(&fixture(), &d.path().join("ws")).unwrap();
        assert!(
            !p.ws.join("solution").exists(),
            "solution stays out of the workspace"
        );
        let before = run_tests(&p).unwrap();
        assert_ne!(
            before.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&before.stdout)
        );
        std::fs::write(p.ws.join(TARGET), &p.solution).unwrap();
        let after = run_tests(&p).unwrap();
        assert_eq!(
            after.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&after.stdout)
        );
        let ev = evidence(&p, &before, &after).unwrap();
        assert_eq!(ev.test_run["exit_code"], 0);
        assert_eq!(
            ev.source_pin["target_blob_sha256"],
            json!(p.target_blob_sha256)
        );
        assert_eq!(
            ev.test_run["target_blob_sha256_after"],
            json!(sha256_hex(p.solution.as_bytes()))
        );
        assert_ne!(ev.tree_commit_after, p.commit);
    }

    #[test]
    fn no_build_output_is_tracked() {
        let d = tempfile::tempdir().unwrap();
        let p = prepare(&fixture(), &d.path().join("ws")).unwrap();
        run_tests(&p).unwrap();
        let tracked = git(&p.ws, &["ls-files"]).unwrap();
        assert!(!tracked.contains("build/"), "{tracked}");
    }
}
