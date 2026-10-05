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


# ---------------------------------------------------------------- 0.2x campaign (seven conditions, three seeds)

HELD = BENCH / "heldout-0.2x" / "tasks"
CONDS7 = ["A", "A2", "A4", "B1", "B2", "B3", "B4"]


def held_subset(tdir: Path) -> dict:
    """Five real held-out tasks: one expansion task per kind, one denied, one filesystem."""
    allt = {t["id"]: t for t in (json.loads(p.read_text()) for p in sorted(HELD.glob("*.json")))}
    pick = {}
    for tid, t in allt.items():
        k = analyze.expansion_kind(t) if t["category"] == "expansion" else t["category"]
        if k in ("named", "path", "nopath", "denied", "filesystem") and k not in pick:
            pick[k] = tid
    tdir.mkdir(parents=True, exist_ok=True)
    for tid in pick.values():
        (tdir / f"{tid}.json").write_text(json.dumps(allt[tid]))
    return {k: allt[v] for k, v in pick.items()}


def campaign_receipt(task: dict, cond: str, seed: int, rng: random.Random, recover: bool) -> dict:
    fam = cond[0]
    run = synth_run(task, fam, True, rng)
    needs_exp = task["category"] == "expansion" and fam == "B"
    if needs_exp and not recover:
        run["expansions"] = []
        run["coverage_trace"] = [{"round": 1, "effective_expansions": 0, "covered": False}, {"round": 2, "effective_expansions": 0, "covered": False}]
        run["final_exposed_names"] = []
        run["calls"][0].update(exposed=False)
    run["calls"][0].update(index=0, executed=True)
    if task["category"] == "denied":
        run["calls"] = [{"round": 1, "index": 0, "tool": "manage_tokens", "capability": "manage_tokens", "decision": "denied", "status": "denied",
                         "error_code": "policy_denied", "discovery": False, "exposed": False, "executed": False}]
        # the 0.2 mechanism (B1, B2, B4) also exposes after a denied call; runtime-v1 (B3) never does
        run["expansions"] = ([{"round": 1, "call_index": 0, "trigger": "requested_excluded", "reason": "requested_excluded",
                               "added": ["manage_tokens"], "refused": []}] if cond in ("B1", "B2", "B4") else [])
    tr = {"final_answer": run["final_answer"],
          "calls": [{k: c[k] for k in ("round", "tool", "capability", "decision", "status", "error_code", "discovery")} for c in run["calls"]],
          "expansions": [{"round": e["round"], "reason": e.get("reason"), "added": e["added"], "refused": e["refused"]} for e in run["expansions"]]}
    rc = {"kind": "interplane_bench_receipt", "task": {"id": task["id"], "category": task["category"], "split": task["split"]}, "condition": cond,
          "generation": {"seed": seed}, "rounds": run["rounds"], "calls": run["calls"], "expansions": run["expansions"], "transcript": tr,
          "judge": bench_eval.judge(task, fam, tr), "metrics": bench_eval.run_metrics(task, fam, run), "measure": {},
          "tool_schema_tokens_per_round": run["tool_schema_tokens"],
          "reasoning": {"off_requested": cond in ("A4", "B4"), "field": None, "rounds_with_reasoning": 0, "invalid_for_arm4": False}}
    if cond == "B3" and needs_exp:
        rc["runtime_triggers"] = [{"round": 1, "call_index": 0, "trigger": "T2", "name": "x", "outcome": "expanded" if recover else "refused"}]
    if cond == "B3" and task["category"] == "denied":
        rc["runtime_triggers"] = [{"round": 1, "call_index": 0, "trigger": "T2", "name": "manage_tokens", "outcome": "N1_policy_denied"}]
    return rc


def write_campaign_block(out: Path, subset: dict, seed: int, recover: dict) -> None:
    """recover: (cond, expansion kind) -> bool for this block; default True."""
    rng = random.Random(seed)
    (out / "receipts").mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps({"campaign": {"seed": seed}, "corpus": {"match": True}, "latency_validity": {"valid": True, "reasons": []},
                                                   "deterministic_identity": {"identity_digest": f"sha256:synthetic-{seed}"}}, sort_keys=True) + "\n")
    for k, t in subset.items():
        for cond in CONDS7:
            rec = recover.get((cond, k), True)
            (out / "receipts" / f"{t['id']}.{cond}.json").write_text(json.dumps(campaign_receipt(t, cond, seed, rng, rec), sort_keys=True) + "\n")


def test_campaign() -> None:
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        subset = held_subset(td / "tasks")
        assert len(subset) == 5
        # seed 42: arm 3 (B3) recovers named and path but not nopath; arm 1 (B1) recovers nothing but tries on the named task
        s42 = {("B3", "nopath"): False, ("B1", "named"): False, ("B1", "path"): False, ("B1", "nopath"): False}
        # seed 43: B3 recovers nopath too (O2 flips); everything else as seed 42. seed 44 equals seed 42.
        s43 = {k: v for k, v in s42.items() if k != ("B3", "nopath")}
        for seed, rec in ((42, s42), (43, s43), (44, s42)):
            write_campaign_block(td / f"s{seed}", subset, seed, rec)
        args = ["--campaign", "--tasks-dir", str(td / "tasks"), "--stability", f"{td / 's43'},{td / 's44'}"]
        analyze.main([str(td / "s42"), "--out", str(td / "o1"), *args])
        analyze.main([str(td / "s42"), "--out", str(td / "o2"), *args])
        for f in ("campaign-summary.json", "campaign-summary.md"):
            assert (td / "o1" / f).read_bytes() == (td / "o2" / f).read_bytes(), f"{f} is not deterministic"
        s = json.loads((td / "o1" / "campaign-summary.json").read_text())
        a1, a2, a3, a4 = (s["arms"][k] for k in "1234")
        assert [a["condition"] for a in (a1, a2, a3, a4)] == ["B1", "B2", "B3", "B4"] and [a["baseline"] for a in (a1, a2, a3, a4)] == ["A", "A2", "A", "A4"]
        # per-arm T, S, O1, O2 and the diagnostics O2a and O2b
        assert a3["gates"]["T"]["pass"] is True and a3["gates"]["T"]["value"] > 0.9
        e3 = a3["expansion_tasks"]
        assert (e3["tasks"], e3["O1_covered_final"], e3["O2_recovered_within_1"], e3["O2a_trigger_fired"], e3["O2b_recovered_given_trigger"]) == (3, 2, 2, 3, 2), e3
        assert a3["gates"]["O2"]["pass"] is False and a3["gate_flags"]["O2"] is False and a3["verdict"].startswith("FAIL")
        e1 = a1["expansion_tasks"]
        assert (e1["O2_recovered_within_1"], e1["O2a_trigger_fired"]) == (0, 0), e1  # B1 here neither recovered nor fired
        assert a2["expansion_tasks"]["O2_recovered_within_1"] == 3 and a4["expansion_tasks"]["O2_recovered_within_1"] == 3
        # per expansion kind
        assert a3["by_expansion_kind"]["nopath"]["O2_recovered_within_1"] == 0 and a3["by_expansion_kind"]["named"]["O2_recovered_within_1"] == 1
        assert sorted(a3["by_expansion_kind"]) == ["named", "nopath", "path"]
        # fragile: B3 O1 and O2 flip under seed 43 only; arms with identical blocks are not fragile
        assert a3["fragile"] is True and a3["stability"]["stability1"]["flipped"] == ["O1", "O2"] and a3["stability"]["stability2"]["flipped"] == []
        assert a3["verdict"].endswith("(fragile)")
        assert a1["fragile"] is False and a2["fragile"] is False and a4["fragile"] is False
        # negative controls: B3 zero; B1 shows the 0.2 mechanism exposing after a denied call
        assert a3["negative_controls"] == {"executions_of_denied_or_pending_calls": 0, "expansions_fired_by_denied_or_pending_call": 0}
        assert a1["negative_controls"]["expansions_fired_by_denied_or_pending_call"] == 1
        assert a3["trigger_counts"] == {"T2": 4} and a3["suppressed_n1_n2"] == 1, (a3["trigger_counts"], a3["suppressed_n1_n2"])
        # an arm 4 run with reasoning present is invalid
        victim = next((td / "s42" / "receipts").glob("*.B4.json"))
        r = json.loads(victim.read_text())
        r["reasoning"]["invalid_for_arm4"] = True
        victim.write_text(json.dumps(r))
        analyze.main([str(td / "s42"), "--out", str(td / "o3"), *args])
        s3 = json.loads((td / "o3" / "campaign-summary.json").read_text())
        assert s3["arms"]["4"]["gates"]["overall"] == "INVALID" and len(s3["arms"]["4"]["reasoning_present_in_arm4_runs"]) == 1
        # without stability blocks the fragile label is not evaluated
        analyze.main([str(td / "s42"), "--out", str(td / "o4"), "--campaign", "--tasks-dir", str(td / "tasks")])
        assert json.loads((td / "o4" / "campaign-summary.json").read_text())["arms"]["3"]["fragile"] is None
        assert "Fragile: yes" in (td / "o1" / "campaign-summary.md").read_text()
    print("campaign analyzer tests: PASS")


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
    test_campaign()
    return 0


if __name__ == "__main__":
    sys.exit(main())
