"""Offline test of bench/tools/analyze.py on a synthetic run (stdlib only; no model, no GPU).

  python3 bench/tools/test_analyze.py                  run the tests
  python3 bench/tools/test_analyze.py --write-fixture  regenerate bench/tools/testdata/synthetic-run

The synthetic run uses real task ids (judge and category come from bench/tasks) with invented,
seeded receipts built through bench_eval (the same judge and metrics code the runner uses). The
checked-in fixture and its expected outputs are what the CI job `bench-structural` re-analyzes.
"""

from __future__ import annotations

import json
import random
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
sys.path.insert(0, str(HERE))
import analyze  # noqa: E402
import bench_eval  # noqa: E402

FIXTURE = HERE / "testdata" / "synthetic-run"
QUAL = ["filesystem-002", "filesystem-003", "rare-002", "expansion-001", "expansion-002", "unknown_tool-001", "wrong_first_tool-001", "ambiguous-002"]
DEV = ["filesystem-001"]
# (A succeeds, B succeeds) per task; one A-only and one B-only loss keep the paired table non-trivial.
PATTERN = {"filesystem-002": (1, 1), "filesystem-003": (1, 1), "rare-002": (1, 0), "expansion-001": (0, 1), "expansion-002": (1, 1),
           "unknown_tool-001": (1, 1), "wrong_first_tool-001": (0, 0), "ambiguous-002": (1, 1), "filesystem-001": (1, 1)}


def tasks() -> dict:
    return {t["id"]: t for t in (json.loads(p.read_text()) for p in (BENCH / "tasks").glob("*.json"))}


def synth_run(task: dict, cond: str, ok: bool, rng: random.Random) -> dict:
    needs_exp = task["category"] == "expansion"
    rounds_n = 2 if cond == "B" and needs_exp else rng.choice([1, 2, 3])
    exposed = 71 if cond == "A" else 9
    cap = (task["required_capabilities"] or [m for g in task["allowed_alternatives"] for m in g[:1]] or ["read_file"])[0]
    rounds = [{"round": i + 1, "exposed_caps_count": exposed + (1 if (cond == "B" and needs_exp and i) else 0),
               "exposed_tools_count": exposed + (1 if cond == "B" else 0) + (1 if (cond == "B" and needs_exp and i) else 0),
               "tools_bytes": 59000 if cond == "A" else 6000, "prompt_tokens": (15800 if cond == "A" else 1700) + 150 * i,
               "completion_tokens": rng.randint(20, 90), "latency_ms": rng.randint(900, 4000), "finish_reason": "stop" if i == rounds_n - 1 else "tool_calls"}
              for i in range(rounds_n)]
    calls = [{"round": 1, "tool": cap, "capability": cap, "decision": "authorized", "status": "ok", "error_code": None, "discovery": False,
              "exposed": not (cond == "B" and needs_exp)}]
    expansions = [{"round": 1, "reason": "requested_excluded", "added": [cap], "refused": []}] if (cond == "B" and needs_exp) else []
    cov = [{"round": 1, "effective_expansions": 0, "covered": not (cond == "B" and needs_exp)}]
    cov.append({"round": 2, "effective_expansions": len(expansions), "covered": True})
    ans = task["judge"]["expected_answer"] if ok and isinstance(task["judge"]["expected_answer"], str) else ("no idea" if not ok else "done")
    return {"final_answer": ans, "ended": "answer", "infra_error": None, "infra_failure": False, "rounds": rounds, "calls": calls,
            "expansions": expansions, "final_exposed_names": [cap] if cond == "B" else [], "coverage_trace": cov,
            "wall_ms": sum(r["latency_ms"] for r in rounds) + 50, "tool_schema_tokens": [9400 if cond == "A" else 780 + 40 * i for i in range(rounds_n)],
            "base_context": 400}


def write_run(out: Path) -> None:
    ts = tasks()
    rng = random.Random(7)
    (out / "receipts").mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps({"kind": "interplane_bench_manifest", "run_id": "synthetic-run", "model": {"id": "synthetic"},
                                                   "deterministic_identity": {"identity_digest": "sha256:synthetic"},
                                                   "stochastic_execution": {"token_stability": {"stable": True, "tools_count_1": 1, "tools_count_2": 1}},
                                                   "concurrent_load": {"other_inference_processes_seen": []}}, indent=2, sort_keys=True) + "\n")
    for tid in QUAL + DEV:
        t = ts[tid]
        for cond, ok in zip("AB", PATTERN[tid]):
            run = synth_run(t, cond, bool(ok), rng)
            tr = {"final_answer": run["final_answer"],
                  "calls": [{k: c[k] for k in ("round", "tool", "capability", "decision", "status", "error_code", "discovery")} for c in run["calls"]],
                  "expansions": run["expansions"]}
            receipt = {"kind": "interplane_bench_receipt", "task": {"id": tid, "category": t["category"], "split": t["split"]}, "condition": cond,
                       "rounds": run["rounds"], "transcript": tr, "judge": bench_eval.judge(t, cond, tr), "metrics": bench_eval.run_metrics(t, cond, run),
                       "measure": {}, "tool_schema_tokens_per_round": run["tool_schema_tokens"]}
            (out / "receipts" / f"{tid}.{cond}.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")


def analyze_to(run: Path, out: Path, extra: list = ()) -> None:
    analyze.main([str(run), "--out", str(out), *extra])


def test() -> None:
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        write_run(td / "run")
        analyze_to(td / "run", td / "o1")
        analyze_to(td / "run", td / "o2")
        for f in ("summary.json", "summary.md", "tasks.csv"):
            assert (td / "o1" / f).read_bytes() == (td / "o2" / f).read_bytes(), f"{f} is not deterministic"
        s = json.loads((td / "o1" / "summary.json").read_text())
        q = s["qual"]
        assert q["paired"]["n_pairs"] == len(QUAL)
        # contingency recomputed independently from the stored verdicts
        succ = {tid: [json.loads((td / "run" / "receipts" / f"{tid}.{c}.json").read_text())["judge"]["success"] for c in "AB"] for tid in QUAL}
        e = sum(a and b for a, b in succ.values())
        f = sum((not a) and b for a, b in succ.values())
        g = sum(a and (not b) for a, b in succ.values())
        assert q["paired"]["contingency"] == {"both": e, "B_only": f, "A_only": g, "neither": len(QUAL) - e - f - g}, q["paired"]["contingency"]
        gt = q["gates"]
        assert gt["complete"] is False and gt["overall"] == "INCOMPLETE"  # 8 of 37 qual tasks
        assert gt["T"]["pass"] is True and gt["T"]["value"] > 0.9
        assert gt["O2"]["uncovered_on_round1"] == 2 and gt["O2"]["pass"] is True
        assert gt["O1"]["pass"] is True
        assert "filesystem-001" not in q["tasks"] and s["dev"]["tasks"] == ["filesystem-001"]
        assert s["diagnostics"]["rejudge_mismatches"] == []
        # exclusion is sensitivity only: the primary gate does not move
        analyze_to(td / "run", td / "o3", ["--exclude", "rare-002", "--exclude-reason", "test"])
        s3 = json.loads((td / "o3" / "summary.json").read_text())
        assert s3["qual"]["gates"] == q["gates"] and s3["qual"]["sensitivity_exclusion"]["excluded"] == ["rare-002"]
        assert len((td / "o1" / "tasks.csv").read_text().splitlines()) == len(QUAL) + len(DEV) + 1
        # quantile edge cases
        assert analyze.dist([]) == {"n": 0} and analyze.dist([5])["median"] == 5 and analyze.dist([1, 3])["p25"] == 1.5
    print("analyze tests: PASS")


def main() -> int:
    if "--write-fixture" in sys.argv:
        if FIXTURE.exists():
            shutil.rmtree(FIXTURE)
        write_run(FIXTURE)
        out = FIXTURE.parent / "synthetic-expected"
        if out.exists():
            shutil.rmtree(out)
        analyze_to(FIXTURE, out)
        print("fixture and expected outputs written")
        return 0
    test()
    return 0


if __name__ == "__main__":
    sys.exit(main())
