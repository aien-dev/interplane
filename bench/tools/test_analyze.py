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
import trust_run  # noqa: E402

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
    (out / "manifest.json").write_text(json.dumps({"campaign": {"seed": seed, "corpus": "0.2x"}, "corpus": {"match": True}, "latency_validity": {"valid": True, "reasons": []},
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


def r1_receipt(cond: str, tid: str = "t-1", *, text: bool = True, query: bool = True, bad_query=None) -> dict:
    """Minimal receipt carrying only what bench_eval.r1_problems reads; discovery call only for B-family arms."""
    r = {"task": {"id": tid}, "condition": cond, "rounds": [{"round": 1}], "calls": []}
    if text:
        r["rounds"][0]["text"] = "assistant text"
    if cond != "A":
        c = {"round": 1, "index": 0, "discovery": True}
        if bad_query is not None:
            c.update(query=bad_query[0], error_code=bad_query[1])
        elif query:
            c["query"] = "serve preset"
        r["calls"].append(c)
    return r


def write_r1_run(out: Path, corpus, receipts: list, *, campaign: bool = True, di_corpus=None) -> Path:
    (out / "receipts").mkdir(parents=True)
    m: dict = {"kind": "x"}
    if campaign:
        m["campaign"] = {"corpus": corpus, "seed": 42} if corpus is not None else {"seed": 42}
    if di_corpus is not None:
        m["deterministic_identity"] = {"campaign": {"corpus": di_corpus}}
    (out / "manifest.json").write_text(json.dumps(m))
    for i, r in enumerate(receipts):
        (out / "receipts" / f"{r['task']['id']}.{r['condition']}.{i}.json").write_text(json.dumps(r))
    return out


def test_r1_scope() -> None:
    tdir = BENCH / "tasks"
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        n = [0]

        def load(corpus, receipts, **kw):
            n[0] += 1
            return analyze.load_run(write_r1_run(td / f"r{n[0]}", corpus, receipts, **kw), tdir)

        def bad(d):
            return sorted((x["task"], x["condition"]) for x in d["r1_invalid"])

        # (e) complete valid receipts in every 0.2y arm pass
        full = [r1_receipt(c, f"t-{c}") for c in ("A", "B3", "B5")]
        assert load("0.2y", full)["r1_invalid"] == []
        # (a) missing round text in arm A is invalid under 0.2y, but valid for the historical 0.2x corpus (original rule: B5 only)
        a_notext = [r1_receipt("A", "t-A", text=False), r1_receipt("B3", "t-B3"), r1_receipt("B5", "t-B5")]
        assert bad(load("0.2y", a_notext)) == [("t-A", "A")]
        # (f) historical 0.2x receipts without R1 fields still validate; B5 is still enforced there
        hist = [r1_receipt(c, f"t-{c}", text=False, query=False) for c in ("A", "B3")]
        assert load("0.2x", hist)["r1_invalid"] == []
        assert bad(load("0.2x", hist + [r1_receipt("B5", "t-B5", text=False)])) == [("t-B5", "B5")]
        # 0.2 layout (no campaign block, conditions A and B only) is legacy and unchanged
        legacy = [r1_receipt("A", "t-A", text=False), r1_receipt("B", "t-B", text=False, query=False)]
        assert load(None, legacy, campaign=False)["r1_invalid"] == []
        # (b) missing discovery queries in B3, (c) in B5
        assert bad(load("0.2y", [r1_receipt("A"), r1_receipt("B3", "t-B3", query=False), r1_receipt("B5", "t-B5")])) == [("t-B3", "B3")]
        assert bad(load("0.2y", [r1_receipt("A"), r1_receipt("B3", "t-B3"), r1_receipt("B5", "t-B5", query=False)])) == [("t-B5", "B5")]
        # (d) malformed recording: raw non-string query is valid only as an invalid_arguments error, in any 0.2y arm
        ok_bad_args = [r1_receipt("A"), r1_receipt("B3", "t-B3", bad_query=(None, "invalid_arguments")), r1_receipt("B5", "t-B5", bad_query=(7, "invalid_arguments"))]
        assert load("0.2y", ok_bad_args)["r1_invalid"] == []
        for arm in ("B3", "B5"):
            rs = [r1_receipt("A"), r1_receipt("B3", "t-B3"), r1_receipt("B5", "t-B5")]
            rs = [r1_receipt(arm, f"t-{arm}", bad_query=(["x"], "ok")) if r["condition"] == arm else r for r in rs]
            assert bad(load("0.2y", rs)) == [(f"t-{arm}", arm)], arm
        # (g) fail closed on identity: never silently historical
        for label, d in (("campaign block without corpus", load(None, full)),
                         ("unknown corpus", load("0.3", full)),
                         ("disagreeing identity", load("0.2y", full, di_corpus="0.2x")),
                         ("no manifest campaign, non-0.2 arms", load(None, full, campaign=False)),
                         ("arm outside the 0.2y list", load("0.2y", full + [r1_receipt("B1", "t-B1")]))):
            assert any(x["task"] == "*" or x["condition"] == "B1" for x in d["r1_invalid"]), label
        assert load("0.2y", full, di_corpus="0.2y")["r1_invalid"] == []
        # (h) one invalid 0.2y receipt refuses the whole run, in build and in the campaign path (non-zero exit)
        badrun = write_r1_run(td / "bad", "0.2y", [r1_receipt("A", "t-A"), r1_receipt("B3", "t-B3", query=False), r1_receipt("B5", "t-B5")])
        for call in (lambda: analyze.build(badrun, tdir, [], ""),
                     lambda: analyze.build_campaign(badrun, [], tdir),
                     lambda: analyze.main([str(badrun), "--out", str(td / "o")])):
            try:
                call()
                raise AssertionError("an invalid 0.2y receipt did not stop scoring")
            except SystemExit as e:
                assert e.code not in (0, None) and "without valid R1 fields" in str(e.code), e.code
        assert not (td / "o").exists()


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


def trust_gate_dir(out: Path, t1: dict, t3_rows: dict, t4_rows: dict, codes: dict | None = None) -> None:
    """Minimal files trust_run.gates reads: verdicts, empty dumps and logs, empty matrices, leg exit codes."""
    if codes is not None:
        (out / "exit-codes.json").write_text(json.dumps(codes, sort_keys=True), encoding="utf-8")
    for name, obj in (("t1-verdicts.json", t1), ("t2-verdicts.json", t1),
                      ("t3-verdicts.json", {"rows": t3_rows, "summary": {"violations": 0, "content_derived": 0}}),
                      ("t4-verdicts.json", {"rows": t4_rows, "summary": {"violations": 0, "content_derived": 0}}),
                      ("n-matrix-t1.json", {}), ("n-matrix-t2.json", {})):
        (out / name).write_text(json.dumps(obj, sort_keys=True), encoding="utf-8")
    for d in ("dump-t1", "dump-t2"):
        (out / d).mkdir()
    for log in ("validate.log", "t1.log"):
        (out / log).write_text("", encoding="utf-8")


def test_trust_run_no_vacuous_gates() -> None:
    """Gates X and E of trust_run must not pass on zero or partial rows (denominator zero is not a pass)."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "empty"
        out.mkdir()
        trust_gate_dir(out, {}, {}, {})
        g = trust_run.gates(out)
        assert g["X"]["pass"] is False, g["X"]
        assert g["E"]["pass"] is False, g["E"]
        assert g["offline_pass"] is False

        rows = trust_run.expected_rows()
        inj = set(trust_run.expected_injection_rows())
        t3s, t4s = trust_run.frozen_subsets()
        full = {n: ({"pass": True, "injection": {"violations": 0, "content_derived": 0}} if n in inj else {"pass": True}) for n in rows}
        sub = lambda names: {n: {"pass": True} for n in names}  # noqa: E731
        out = Path(tmp) / "full"
        out.mkdir()
        trust_gate_dir(out, full, sub(t3s), sub(t4s))
        g = trust_run.gates(out)
        assert len(rows) == 106 and len(inj) == 44, (len(rows), len(inj))
        assert g["X"]["pass"] is True and g["E"]["pass"] is True, (g["X"], g["E"])

        one_inj = sorted(inj)[0]
        partial = {n: v for n, v in full.items() if n != one_inj}
        out = Path(tmp) / "partial"
        out.mkdir()
        trust_gate_dir(out, partial, sub(t3s), sub(t4s))
        g = trust_run.gates(out)
        assert g["X"]["pass"] is False and g["X"]["missing_rows"] == [one_inj], g["X"]
        assert g["E"]["pass"] is False, g["E"]

        out = Path(tmp) / "t4-short"
        out.mkdir()
        trust_gate_dir(out, full, sub(t3s), sub([n for n in t4s if n != "injection/01-workspace-write"]))
        assert trust_run.gates(out)["E"]["pass"] is False

        # Right count, wrong cases: swap one frozen T4 injection case for one outside the subset.
        outside = sorted({"injection/" + p.stem for p in (trust_run.ROOT / "conformance" / "fixtures" / "injection").glob("*.json")}
                         - set(t4s))
        assert outside, "need an injection fixture outside the T4 subset"
        swapped = [n for n in t4s if n != "injection/01-workspace-write"] + [outside[0]]
        out = Path(tmp) / "t4-swapped"
        out.mkdir()
        trust_gate_dir(out, full, sub(t3s), sub(swapped))
        e = trust_run.gates(out)["E"]
        assert e["pass"] is False and e["systems"]["T4"]["injection_cases"] == e["systems"]["T4"]["expected_injection_cases"], e
        assert e["systems"]["T4"]["missing_injection_cases"] == ["injection/01-workspace-write"], e
        assert e["systems"]["T4"]["unexpected_injection_cases"] == [outside[0]], e

        # F: the trust-digest and fixture-validation legs must have run and exited 0.
        ok = dict.fromkeys(("digest", "n1", "n2", "t1", "t2", "t3", "t4", "validate"), 0)
        for name, codes in (("no-codes", None), ("digest-fail", {**ok, "digest": 1}), ("validate-fail", {**ok, "validate": 1}),
                            ("codes-ok", ok)):
            out = Path(tmp) / name
            out.mkdir()
            trust_gate_dir(out, full, sub(t3s), sub(t4s), codes)
            assert trust_run.gates(out)["F"]["pass"] is (name == "codes-ok"), name
    print("test_trust_run_no_vacuous_gates: ok")


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
    test_r1_scope()
    print("r1 scope tests: PASS")
    test_trust_run_no_vacuous_gates()
    return 0


if __name__ == "__main__":
    sys.exit(main())
