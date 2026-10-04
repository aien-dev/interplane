# Run manifests record pre-squash commits

Bench run manifests store `interplane.commit`, the commit the runner was on when the run started. Pull requests here
are squash-merged, so that commit is a branch commit that is not an ancestor of `main`. Content can be compared
(`git diff <manifest commit> <main commit> -- <path>` while the branch commit is still fetchable), but the manifest
commit id alone cannot be checked out from `main`. This file is append-only: add a row per run, never edit old rows,
and never edit files under `bench/runs/<run>/`.

| Run | Manifest `interplane.commit` (pre-squash) | Squash-merged commit on `main` | PR |
|---|---|---|---|
| qual-20261004T2207Z | `661ea669e0e8742f5556251440cd4fc7c88e5bfe` | `2bad9ac` (`git log` of `bench/runs/qual-20261004T2207Z` shows it as the commit that added the run) | #11 |

Source for the manifest value: `bench/runs/qual-20261004T2207Z/manifest.json`. Background:
`bench/runs/qual-20261004T2207Z-analysis/REPRODUCTION.md` section 4.

## Toolchain and dependency pins (added with the pin lane)

- Rust: `rust-toolchain.toml` (1.99.0, the compiler CI used on 2026-10-04); CI passes the same `toolchain:` value.
- Python CI packages: `constraints/ci-python.txt`, used with `pip install -c`. Python is `3.12.14`.
- GitHub Actions: pinned to full commit SHAs, tag in a comment.
- AIEN adapter siblings: `adapters/aien/PINS`, read by `adapters/aien/setup-siblings.sh`.
- Odysseus itself stays pinned at `2992bf6` in CI; its own `requirements.txt` is unversioned upstream, so
  `constraints/ci-python.txt` fixes the versions it resolves to.
