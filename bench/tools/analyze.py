"""Re-runnable analyzer for an INTERPLANE 0.2 bench run directory (stdlib only; no inference).

  python3 bench/tools/analyze.py bench/runs/<run-id> [--out DIR] [--exclude ID,ID --exclude-reason TEXT]

Reads manifest.json and receipts/*.json, and writes summary.json, summary.md and tasks.csv.
Reports every metric of PROTOCOL-0.2.md section 4 as distributions, the paired success table
with the Newcombe method-10 interval and the exact McNemar test (bench/tools/stats.py), paired
deltas with a seeded percentile bootstrap, and the gates T, S, O1 and O2 exactly as pre-registered
(section 6) on the qualification tasks. Dev tasks never enter a gate; they are listed and
summarized separately. The primary gate always uses every qualification task: ``--exclude`` adds
a separately labelled sensitivity section and never changes the primary result (section 6: no
qual task is dropped after results are seen).

Deterministic: the same input gives byte-identical output (fixed bootstrap seed, fixed float
rounding, sorted keys, no timestamps, no environment data).

Campaign mode (PROTOCOL-0.2x.md sections 4, 6, 8, 11):

  python3 bench/tools/analyze.py <seed-42 run dir> --campaign [--stability <seed-43 dir>,<seed-44 dir>] [--out DIR]

reads the seven-condition run of the held-out set (receipts ``<task>.<A|A2|A4|B1|B2|B3|B4>.json``)
and writes campaign-summary.json and campaign-summary.md: per arm (1 to 4) the gate T, S, O1, O2 on
seed 42 against its matched baseline, the diagnostics O2a and O2b, O1 and O2 per expansion kind,
trigger counts, refusals, the negative controls, and the label ``fragile`` when a gate criterion
of the arm flips under seed 43 or 44. The 0.2 outputs above are unchanged.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import math
import random
import statistics
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import bench_eval  # noqa: E402
import stats  # noqa: E402

BENCH = HERE.parent
BOOT_SEED = 20261004
BOOT_N = 10000
ROUND = 6
GATE_T, GATE_O1, GATE_O2 = 0.70, 0.95, 0.90
# PROTOCOL-0.2x.md section 4: arm -> (matched baseline condition, arm condition)
ARMS = {"1": ("A", "B1"), "2": ("A2", "B2"), "3": ("A", "B3"), "4": ("A4", "B4")}
CRITERIA = ("T", "S", "O1", "O2")
HELDOUT_TASKS = BENCH / "heldout-0.2x" / "tasks"


def rnd(x):
    if isinstance(x, float):
        if math.isnan(x) or math.isinf(x):
            return None
        return round(x, ROUND)
    if isinstance(x, dict):
        return {k: rnd(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [rnd(v) for v in x]
    return x


# ------------------------------------------------------------------------------- distributions

def quantiles(vals: list):
    """(p25, p50, p75, p95) by linear interpolation (statistics.quantiles, method inclusive)."""
    if not vals:
        return None, None, None, None
    if len(vals) == 1:
        return vals[0], vals[0], vals[0], vals[0]
    q4 = statistics.quantiles(vals, n=4, method="inclusive")
    q20 = statistics.quantiles(vals, n=20, method="inclusive")
    return q4[0], q4[1], q4[2], q20[18]


def dist(vals: list, timing: bool = False) -> dict:
    vals = sorted(v for v in vals if v is not None)
    if not vals:
        return {"n": 0}
    p25, p50, p75, p95 = quantiles(vals)
    d = {"n": len(vals), "median": p50, "p25": p25, "p75": p75, "min": vals[0], "max": vals[-1], "mean": sum(vals) / len(vals)}
    if timing:
        d["p50"], d["p95"] = p50, p95
    return d


def reduction(a, b):
    return None if not a else 1 - b / a


def boot_ci(deltas: list, seed: int = BOOT_SEED, n_boot: int = BOOT_N) -> dict:
    n = len(deltas)
    if n == 0:
        return {"n": 0}
    rng = random.Random(seed)
    means = sorted(sum(deltas[rng.randrange(n)] for _ in range(n)) / n for _ in range(n_boot))
    return {"n": n, "mean": sum(deltas) / n, "median": statistics.median(deltas),
            "ci95": [means[int(0.025 * n_boot)], means[int(0.975 * n_boot) - 1]], "resamples": n_boot, "seed": seed}


# ------------------------------------------------------------------------------- loading

R1_ALWAYS = ("B5",)  # historical rule: only B5 receipts carry R1 (PREREG-0.2y condition B5, #64)
# PREREG-0.2y section 4: a run labelled 0.2y must satisfy R1 in every one of these arms and use no other arm.
R1_CAMPAIGNS = {"0.2y": ("A", "B3", "B5")}
# Historical corpora: the weaker R1 rule applies only to a run PROVEN to be that corpus. The label in the manifest is
# self-declared and proves nothing. Proof = the manifest records the frozen tasks_digest of that corpus (the value in its
# CORPUS-DIGEST.txt, written by validate.py/run_bench.check_corpus) AND every receipt's task id is a task of that corpus.
# Arm lists are the historical ones: 0.2 ran A,B (PROTOCOL-0.2.md); 0.2x ran CAMPAIGN_CONDITIONS (run_bench.py; B5 is
# "explicit only, never part of CAMPAIGN_CONDITIONS", #64), so B5 in a historical run is refused.
HISTORICAL = {
    "0.2": {"digest": BENCH / "CORPUS-DIGEST.txt", "tasks": BENCH / "tasks", "arms": ("A", "B")},
    "0.2x": {"digest": BENCH / "heldout-0.2x" / "CORPUS-DIGEST.txt", "tasks": BENCH / "heldout-0.2x" / "tasks",
             "arms": ("A", "A2", "A4", "B1", "B2", "B3", "B4")},
}
DIGEST_0_2Y = BENCH / "heldout-0.2y" / "CORPUS-DIGEST.txt"  # absent until the 0.2y corpus is frozen


def frozen_tasks_digest(path: Path):
    if not path.exists():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("tasks_digest "):
            return line.split()[1]
    return None


def corpus_task_ids(tasks_dir: Path) -> set:
    return {json.loads(p.read_text(encoding="utf-8"))["id"] for p in tasks_dir.glob("*.json")}


def r1_scope(manifest: dict, conditions: set, task_ids: set) -> tuple:
    """Returns (arms that need R1, or None for every arm; problems). Problems refuse the run (fail closed).
    Anything not proven historical gets the strictest rule: R1 in every arm."""
    camp = manifest.get("campaign")
    corpus_blk = manifest.get("corpus") if isinstance(manifest.get("corpus"), dict) else {}
    label = camp.get("corpus") if isinstance(camp, dict) else None
    if camp is not None and (not isinstance(label, str) or not label):
        return None, ["campaign block has no corpus identity"]
    di = ((manifest.get("deterministic_identity") or {}).get("campaign") or {}).get("corpus")
    if camp is not None and di is not None and di != label:
        return None, [f"campaign.corpus {label!r} disagrees with deterministic_identity.campaign.corpus {di!r}"]
    digest = (corpus_blk.get("frozen") or {}).get("tasks_digest")
    recomputed = (corpus_blk.get("recomputed") or {}).get("tasks_digest")
    proven = next((k for k, h in HISTORICAL.items()
                   if digest and digest == recomputed == frozen_tasks_digest(h["digest"]) and corpus_blk.get("match") is True), None)
    if proven is not None:
        h = HISTORICAL[proven]
        if label is not None and label != proven:
            return None, [f"manifest label {label!r} contradicts the frozen task-set digest of corpus {proven}"]
        if label is None and proven != "0.2":
            return None, [f"manifest without campaign block carries the {proven} digest"]
        outside = sorted(task_ids - corpus_task_ids(h["tasks"]))
        if outside:  # e.g. a dev run with --allow-nonfrozen and its own task dir: not the frozen corpus, so strict
            return None, []
        extra = sorted(conditions - set(h["arms"]))
        if extra:
            return None, [f"condition(s) {extra} were not part of historical corpus {proven} (arms {list(h['arms'])})"]
        return R1_ALWAYS, []
    # not proven historical from here on: strictest rule (R1 in every arm)
    if label in R1_CAMPAIGNS:
        arms = R1_CAMPAIGNS[label]
        extra = sorted(conditions - set(arms))
        problems = [f"condition(s) {extra} are not in the {label} arm list {list(arms)}"] if extra else []
        frozen_y = frozen_tasks_digest(DIGEST_0_2Y)
        if frozen_y is not None and not (digest == recomputed == frozen_y):
            problems.append(f"{label} manifest does not record the frozen 0.2y tasks_digest")
        return None, problems
    if camp is None:
        legacy_ok = (conditions <= set(HISTORICAL["0.2"]["arms"]) and task_ids <= corpus_task_ids(HISTORICAL["0.2"]["tasks"]))
        if legacy_ok and not (digest or recomputed):
            return R1_ALWAYS, []  # legacy 0.2 layout (also no manifest): frozen 0.2 task ids, 0.2 arms, no other identity claimed
        return None, ["no provable corpus identity (no campaign block, digest absent or not frozen) for these tasks or arms"]
    # labelled campaign that is not 0.2y and not proven historical (offline dev runs on custom task dirs): strict R1 only
    return None, []


def load_run(run_dir: Path, tasks_dir: Path) -> dict:
    manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8")) if (run_dir / "manifest.json").exists() else {}
    rec, r1_invalid, raw = {}, [], []
    for p in sorted((run_dir / "receipts").glob("*.json")):
        r = json.loads(p.read_text(encoding="utf-8"))
        rec.setdefault(r["task"]["id"], {})[r["condition"]] = r
        raw.append(r)
    required, id_problems = r1_scope(manifest, {r["condition"] for r in raw}, {r["task"]["id"] for r in raw})
    if id_problems:  # fail closed: an untrustworthy identity must not downgrade R1 enforcement
        r1_invalid.append({"task": "*", "condition": "*", "problems": id_problems})
    for r in raw:
        if (required is None or r["condition"] in required) and not bench_eval.r1_valid(r):  # PREREG-0.2y: missing R1 fields are invalid
            r1_invalid.append({"task": r["task"]["id"], "condition": r["condition"], "problems": bench_eval.r1_problems(r)})
    tasks = {}
    for p in sorted(tasks_dir.glob("*.json")):
        t = json.loads(p.read_text(encoding="utf-8"))
        tasks[t["id"]] = t
    return {"manifest": manifest, "receipts": rec, "tasks": tasks, "r1_invalid": r1_invalid}


def refuse_r1_invalid(data: dict, label: str) -> None:
    """PREREG-0.2y validity: no run with an unrecorded query or text counts. The analyzer refuses the
    whole run rather than scoring around it."""
    if data["r1_invalid"]:
        raise SystemExit(f"{label}: {len(data['r1_invalid'])} receipt(s) or identity check(s) without valid R1 fields, run not scored: "
                         + json.dumps(data["r1_invalid"][:5]))


def pair_class(a: bool, b: bool) -> str:
    return "both" if a and b else ("B_only" if b else ("A_only" if a else "neither"))


# ------------------------------------------------------------------------------- one task set

def metric_dists(rows: list, cond: str) -> dict:
    """Distributions per condition over the tasks of ``rows`` (each row has A and B receipts)."""
    ms = [r[cond]["metrics"] for r in rows if cond in r]
    runs = [r[cond] for r in rows if cond in r]
    g = lambda k: [m[k] for m in ms]  # noqa: E731
    calls_all = sum(m["calls_total"] for m in ms)
    unn_all = sum(m["unnecessary_calls"] for m in ms)
    unk_all = sum(m["unknown_calls"] for m in ms)
    per_run_unn = [m["unnecessary_calls"] / m["calls_total"] for m in ms if m["calls_total"]]
    per_run_unk = [m["unknown_calls"] / m["calls_total"] for m in ms if m["calls_total"]]
    err_runs = [m for m in ms if m["execution_errors"] >= 1]
    inj = [m["injection_attempt"] for m in ms if m["injection_attempt"] is not None]
    dec = {k: sum(m["decisions"][k] for m in ms) for k in ("authorized", "denied", "requires_approval", "not_found", "invalid")}
    return {
        "n_runs": len(ms),
        "success": {"count": sum(1 for r in runs if r["judge"]["success"]), "rate": (sum(1 for r in runs if r["judge"]["success"]) / len(runs)) if runs else None},
        "exposed_tools_first": dist(g("exposed_tools_first")),
        "exposed_tools_max": dist(g("exposed_tools_max")),
        "exposed_capabilities_first": dist(g("exposed_capabilities_first")),
        "tool_schema_bytes_per_round_mean": dist(g("tool_schema_bytes_per_round_mean")),
        "tool_schema_tokens_per_round_mean": dist(g("tool_schema_tokens_per_round_mean")),
        "tool_schema_tokens_sum": dist(g("tool_schema_tokens_sum")),
        "first_turn_prompt_tokens": dist(g("first_turn_prompt_tokens")),
        "total_prompt_tokens": dist(g("total_prompt_tokens")),
        "total_completion_tokens": dist(g("total_completion_tokens")),
        "base_context_tokens": dist(g("base_context_tokens")),
        "rounds": dist(g("rounds")),
        "wall_ms": dist(g("wall_ms"), timing=True),
        "model_ms": dist(g("model_ms"), timing=True),
        "calls_total": {"sum": calls_all, **dist(g("calls_total"))},
        "unnecessary_call_rate": {"pooled": (unn_all / calls_all) if calls_all else None, "calls": unn_all, "per_run": dist(per_run_unn)},
        "unknown_call_rate": {"pooled": (unk_all / calls_all) if calls_all else None, "calls": unk_all, "per_run": dist(per_run_unk)},
        "unexposed_calls": {"sum": sum(g("unexposed_calls")), **dist(g("unexposed_calls"))},
        "discovery_calls": {"sum": sum(g("discovery_calls"))},
        "decisions": dec,
        "rejected_results": sum(g("rejected_results")),
        "execution_errors": sum(g("execution_errors")),
        "recovery": {"runs_with_error": len(err_runs), "recovered": sum(1 for m in err_runs if m["recovered_after_error"]),
                     "rate": (sum(1 for m in err_runs if m["recovered_after_error"]) / len(err_runs)) if err_runs else None},
        "expansions_per_run": dist(g("expansions")),
        "effective_expansions_per_run": dist(g("effective_expansions")),
        "round_requirements_first_covered": dist([m["covered_at_round"] for m in ms if m["missing_required_first"]]),
        "missing_required_first": {"count": sum(1 for m in ms if m["missing_required_first"]), "of": sum(1 for m in ms if m["missing_required_first"] is not None)},
        "missing_required_final": {"count": sum(1 for m in ms if m["missing_required_final"]), "of": sum(1 for m in ms if m["missing_required_final"] is not None)},
        "injection_attempt": {"runs": len(inj), "attempted": sum(1 for x in inj if x), "rate": (sum(1 for x in inj if x) / len(inj)) if inj else None},
        "infra_failures": sum(1 for m in ms if m["infra_failure"]),
        "empty_final_answers": sum(1 for r in runs if r["transcript"]["final_answer"] == ""),
        "no_final_answer": sum(1 for r in runs if r["transcript"]["final_answer"] is None),
        "finish_reason_length_rounds": sum(1 for r in runs for rd in r["rounds"] if rd.get("finish_reason") == "length"),
    }


def paired(rows: list) -> dict:
    full = [r for r in rows if "A" in r and "B" in r]
    out = {"n_pairs": len(full)}
    if not full:
        return out
    cls = [pair_class(r["A"]["judge"]["success"], r["B"]["judge"]["success"]) for r in full]
    e, f, g_, h = (cls.count(x) for x in ("both", "B_only", "A_only", "neither"))
    out["contingency"] = {"both": e, "B_only": f, "A_only": g_, "neither": h}
    sg = stats.success_gate(e, f, g_, h)
    out["success_delta"] = {"theta": sg["theta"], "ci95_newcombe10": sg["ci95"], "mcnemar_exact_p": sg["mcnemar_p"],
                            "non_inferior": sg["non_inferior"], "equivalent": sg["equivalent"], "margin": stats.MARGIN}
    keys = [("rounds", "rounds"), ("first_turn_prompt_tokens", "first_turn_prompt_tokens"), ("total_prompt_tokens", "total_prompt_tokens"),
            ("total_completion_tokens", "total_completion_tokens"), ("tool_schema_tokens_per_round_mean", "tool_schema_tokens_per_round_mean"),
            ("calls_total", "calls_total"), ("unnecessary_calls", "unnecessary_calls"), ("wall_ms", "wall_ms"), ("model_ms", "model_ms")]
    deltas = {}
    for name, k in keys:
        d = [r["B"]["metrics"][k] - r["A"]["metrics"][k] for r in full if r["A"]["metrics"][k] is not None and r["B"]["metrics"][k] is not None]
        deltas[name] = boot_ci(d)
    out["paired_deltas_B_minus_A"] = deltas
    red = lambda k: [reduction(r["A"]["metrics"][k], r["B"]["metrics"][k]) for r in full if r["A"]["metrics"][k]]  # noqa: E731
    out["reductions_1_minus_B_over_A"] = {
        "exposed_tools_first": dist(red("exposed_tools_first")),
        "exposed_capabilities_first": dist(red("exposed_capabilities_first")),
        "tool_schema_tokens_per_round_mean": dist(red("tool_schema_tokens_per_round_mean")),
        "tool_schema_bytes_per_round_mean": dist(red("tool_schema_bytes_per_round_mean")),
        "first_turn_prompt_tokens": dist(red("first_turn_prompt_tokens")),
        "total_prompt_tokens": dist(red("total_prompt_tokens")),
    }
    return out


def gates(rows: list, expected_n: int) -> dict:
    full = [r for r in rows if "A" in r and "B" in r]
    n = len(full)
    res: dict = {"n_pairs": n, "expected_qual_pairs": expected_n, "complete": n == expected_n}
    # T
    rs = [reduction(r["A"]["metrics"]["tool_schema_tokens_per_round_mean"], r["B"]["metrics"]["tool_schema_tokens_per_round_mean"])
          for r in full if r["A"]["metrics"]["tool_schema_tokens_per_round_mean"] and r["B"]["metrics"]["tool_schema_tokens_per_round_mean"] is not None]
    med = statistics.median(rs) if rs else None
    res["T"] = {"statistic": "median(1 - B/A per-round mean tool-schema tokens)", "value": med, "threshold": GATE_T, "n": len(rs),
                "pass": (med >= GATE_T) if med is not None else None}
    # S
    if n:
        e = sum(1 for r in full if r["A"]["judge"]["success"] and r["B"]["judge"]["success"])
        f = sum(1 for r in full if not r["A"]["judge"]["success"] and r["B"]["judge"]["success"])
        g_ = sum(1 for r in full if r["A"]["judge"]["success"] and not r["B"]["judge"]["success"])
        h = n - e - f - g_
        sg = stats.success_gate(e, f, g_, h)
        res["S"] = {"table": {"e": e, "f": f, "g": g_, "h": h}, "theta": sg["theta"], "ci95": sg["ci95"], "mcnemar_p": sg["mcnemar_p"],
                    "margin": -stats.MARGIN, "pass": sg["non_inferior"], "equivalence_two_sided": sg["equivalent"]}
    else:
        res["S"] = {"pass": None}
    # O1 and O2 (condition B)
    need = [r for r in full if bench_eval.requirements_exist(r["B"]["task_def"])] if n and "task_def" in full[0]["B"] else full
    o1_unc = [r["B"]["task"]["id"] for r in need if r["B"]["metrics"]["missing_required_final"]]
    res["O1"] = {"tasks_with_requirements": len(need), "uncovered_final": len(o1_unc), "uncovered_tasks": sorted(o1_unc),
                 "rate_covered": ((len(need) - len(o1_unc)) / len(need)) if need else None, "threshold": GATE_O1,
                 "pass": (((len(need) - len(o1_unc)) / len(need)) >= GATE_O1) if need else None}
    miss1 = [r for r in need if r["B"]["metrics"]["missing_required_first"]]
    rec1 = [r for r in miss1 if r["B"]["metrics"]["covered_after_effective_expansions"] is not None and r["B"]["metrics"]["covered_after_effective_expansions"] <= 1]
    res["O2"] = {"uncovered_on_round1": len(miss1), "covered_within_1_expansion": len(rec1),
                 "not_recovered_tasks": sorted(r["B"]["task"]["id"] for r in miss1 if r not in rec1),
                 "rate": (len(rec1) / len(miss1)) if miss1 else None, "threshold": GATE_O2,
                 "pass": (len(rec1) / len(miss1) >= GATE_O2) if miss1 else None,
                 "note": "no task was uncovered on round 1: vacuous" if not miss1 else ""}
    flags = [res[k]["pass"] for k in ("T", "S", "O1", "O2")]
    res["overall"] = "INCOMPLETE" if (not res["complete"] or any(f is None for f in flags)) else ("PASS" if all(flags) else "FAIL")
    return res


# ------------------------------------------------------------------------------- build

def build(run_dir: Path, tasks_dir: Path, exclude: list, exclude_reason: str) -> dict:
    data = load_run(run_dir, tasks_dir)
    refuse_r1_invalid(data, run_dir.name)
    tasks, rec = data["tasks"], data["receipts"]
    rows = []
    for tid in sorted(rec):
        row = dict(rec[tid])
        t = tasks.get(tid)
        for c in row.values():
            c["task_def"] = t
        row["_task"] = t
        row["_id"] = tid
        rows.append(row)
    split_of = lambda r: (r["_task"]["split"] if r["_task"] else "unknown")  # noqa: E731
    groups = {s: [r for r in rows if split_of(r) == s] for s in ("qual", "dev", "unknown")}
    expected_qual = sum(1 for t in tasks.values() if t["split"] == "qual")
    out: dict = {"kind": "interplane_bench_summary", "run_id": data["manifest"].get("run_id", run_dir.name),
                 "manifest_identity_digest": (data["manifest"].get("deterministic_identity") or {}).get("identity_digest"),
                 "model": (data["manifest"].get("model") or {}).get("id"),
                 "bootstrap": {"seed": BOOT_SEED, "resamples": BOOT_N, "method": "percentile, resampling task pairs, mean of B minus A"}}
    # diagnostics
    mism, empty_ids = [], []
    anomalies, ft_mismatch = [], []
    for r in rows:
        for c in ("A", "B"):
            if c in r and r["_task"]:
                again = bench_eval.judge(r["_task"], c, r[c]["transcript"]) if not r[c]["judge"].get("infra_failure") else r[c]["judge"]
                if again != r[c]["judge"]:
                    mism.append(f"{r['_id']}.{c}")
            if c in r:
                rd = r[c]["rounds"]
                if len(rd) >= 2 and rd[0]["prompt_tokens"] is not None and rd[1]["prompt_tokens"] is not None and rd[1]["prompt_tokens"] <= rd[0]["prompt_tokens"]:
                    anomalies.append(f"{r['_id']}.{c}")
                tk = (r[c].get("measure") or {}).get("tokens")
                if tk and rd and rd[0]["prompt_tokens"] is not None:
                    want = tk["first_turn_prompt"]["full" if c == "A" else "selected"]
                    if want != rd[0]["prompt_tokens"]:
                        ft_mismatch.append(f"{r['_id']}.{c}:{rd[0]['prompt_tokens']}vs{want}")
    out["diagnostics"] = {
        "rejudge_mismatches": sorted(mism), "rounds_where_prompt_tokens_did_not_grow_round2": sorted(anomalies),
        "first_turn_real_vs_measured_mismatch": sorted(ft_mismatch),
        "tasks_without_both_conditions": sorted(r["_id"] for r in rows if not ("A" in r and "B" in r)),
        "unknown_split_tasks": sorted(r["_id"] for r in groups["unknown"]),
        "token_stability": (data["manifest"].get("stochastic_execution") or {}).get("token_stability"),
        "concurrent_load_other_processes": (data["manifest"].get("concurrent_load") or {}).get("other_inference_processes_seen"),
    }
    for split in ("qual", "dev"):
        g = groups[split]
        if not g:
            continue
        sec = {"tasks": sorted(r["_id"] for r in g), "paired": paired(g), "A": metric_dists(g, "A"), "B": metric_dists(g, "B")}
        cats = sorted({r["_task"]["category"] for r in g})
        sec["by_category"] = {c: {cond: {"success": sum(1 for r in g if r["_task"]["category"] == c and cond in r and r[cond]["judge"]["success"]),
                                         "n": sum(1 for r in g if r["_task"]["category"] == c and cond in r)} for cond in ("A", "B")} for c in cats}
        if split == "qual":
            sec["gates"] = gates(g, expected_qual)
            ex = [e for e in exclude if e]
            if ex:
                kept = [r for r in g if r["_id"] not in ex]
                sec["sensitivity_exclusion"] = {"label": "SENSITIVITY ONLY: not the pre-registered gate", "excluded": sorted(ex), "reason": exclude_reason,
                                                "gates": gates(kept, expected_qual - len(ex)), "paired": paired(kept)}
        else:
            sec["note"] = "dev tasks are excluded from every gate statistic"
        out[split] = sec
    out["failures"] = sorted(
        [{"task": r["_id"], "condition": c, "split": split_of(r), "failed_checks": r[c]["judge"]["failed"]}
         for r in rows for c in ("A", "B") if c in r and not r[c]["judge"]["success"]],
        key=lambda x: (x["task"], x["condition"]))
    out["_rows"] = rows
    return out


# ------------------------------------------------------------------------------- outputs

CSV_COLS = ["task", "category", "split", "success_A", "success_B", "pair", "rounds_A", "rounds_B", "exposed_tools_first_A", "exposed_tools_first_B",
            "exposed_tools_max_B", "schema_tokens_mean_A", "schema_tokens_mean_B", "schema_reduction", "first_turn_prompt_A", "first_turn_prompt_B",
            "total_prompt_A", "total_prompt_B", "completion_A", "completion_B", "wall_ms_A", "wall_ms_B", "calls_A", "calls_B", "unnecessary_A", "unnecessary_B",
            "unknown_A", "unknown_B", "unexposed_B", "discovery_B", "expansions_B", "effective_expansions_B", "missing_first_B", "missing_final_B",
            "covered_round_B", "exec_errors_A", "exec_errors_B", "denied_A", "denied_B", "approval_A", "approval_B", "judge_failed_A", "judge_failed_B"]


def csv_rows(rows: list) -> list:
    out = []
    for r in rows:
        a, b = r.get("A"), r.get("B")
        ma, mb = (a or {}).get("metrics"), (b or {}).get("metrics")
        g = lambda m, k: (m[k] if m else "")  # noqa: E731
        both = a and b
        sch = reduction(ma["tool_schema_tokens_per_round_mean"], mb["tool_schema_tokens_per_round_mean"]) if both and ma["tool_schema_tokens_per_round_mean"] and mb["tool_schema_tokens_per_round_mean"] is not None else ""
        row = {
            "task": r["_id"], "category": r["_task"]["category"] if r["_task"] else "", "split": r["_task"]["split"] if r["_task"] else "",
            "success_A": int(a["judge"]["success"]) if a else "", "success_B": int(b["judge"]["success"]) if b else "",
            "pair": pair_class(a["judge"]["success"], b["judge"]["success"]) if both else "",
            "rounds_A": g(ma, "rounds"), "rounds_B": g(mb, "rounds"),
            "exposed_tools_first_A": g(ma, "exposed_tools_first"), "exposed_tools_first_B": g(mb, "exposed_tools_first"), "exposed_tools_max_B": g(mb, "exposed_tools_max"),
            "schema_tokens_mean_A": g(ma, "tool_schema_tokens_per_round_mean"), "schema_tokens_mean_B": g(mb, "tool_schema_tokens_per_round_mean"), "schema_reduction": sch,
            "first_turn_prompt_A": g(ma, "first_turn_prompt_tokens"), "first_turn_prompt_B": g(mb, "first_turn_prompt_tokens"),
            "total_prompt_A": g(ma, "total_prompt_tokens"), "total_prompt_B": g(mb, "total_prompt_tokens"),
            "completion_A": g(ma, "total_completion_tokens"), "completion_B": g(mb, "total_completion_tokens"),
            "wall_ms_A": g(ma, "wall_ms"), "wall_ms_B": g(mb, "wall_ms"), "calls_A": g(ma, "calls_total"), "calls_B": g(mb, "calls_total"),
            "unnecessary_A": g(ma, "unnecessary_calls"), "unnecessary_B": g(mb, "unnecessary_calls"), "unknown_A": g(ma, "unknown_calls"), "unknown_B": g(mb, "unknown_calls"),
            "unexposed_B": g(mb, "unexposed_calls"), "discovery_B": g(mb, "discovery_calls"), "expansions_B": g(mb, "expansions"),
            "effective_expansions_B": g(mb, "effective_expansions"),
            "missing_first_B": "" if not mb or mb["missing_required_first"] is None else int(mb["missing_required_first"]),
            "missing_final_B": "" if not mb or mb["missing_required_final"] is None else int(mb["missing_required_final"]),
            "covered_round_B": "" if not mb or mb["covered_at_round"] is None else mb["covered_at_round"],
            "exec_errors_A": g(ma, "execution_errors"), "exec_errors_B": g(mb, "execution_errors"),
            "denied_A": ma["decisions"]["denied"] if ma else "", "denied_B": mb["decisions"]["denied"] if mb else "",
            "approval_A": ma["decisions"]["requires_approval"] if ma else "", "approval_B": mb["decisions"]["requires_approval"] if mb else "",
            "judge_failed_A": ";".join(a["judge"]["failed"]) if a else "", "judge_failed_B": ";".join(b["judge"]["failed"]) if b else "",
        }
        out.append({k: (round(v, ROUND) if isinstance(v, float) else v) for k, v in row.items()})
    return out


def fmt(x, nd=3):
    if x is None:
        return "n/a"
    if isinstance(x, bool):
        return "yes" if x else "no"
    if isinstance(x, float):
        return f"{x:.{nd}f}"
    return str(x)


def dline(d: dict, nd=1) -> str:
    if not d or not d.get("n"):
        return "n/a"
    s = f"median {fmt(d['median'], nd)} (p25 {fmt(d['p25'], nd)}, p75 {fmt(d['p75'], nd)}, min {fmt(d['min'], nd)}, max {fmt(d['max'], nd)})"
    if "p95" in d:
        s += f"; p50 {fmt(d['p50'], nd)}, p95 {fmt(d['p95'], nd)}"
    return s


def md(summary: dict) -> str:
    L: list = []
    L.append(f"# Bench summary: {summary['run_id']}")
    L.append("")
    L.append(f"Model `{summary['model']}`. Identity digest `{summary['manifest_identity_digest']}`. Units: tokens are `tokens_model_reported`, times are milliseconds.")
    d = summary["diagnostics"]
    L.append(f"Diagnostics: re-judge mismatches {len(d['rejudge_mismatches'])}; round-2 prompt not larger than round 1 in {len(d['rounds_where_prompt_tokens_did_not_grow_round2'])} runs; "
             f"first-turn real vs measured mismatches {len(d['first_turn_real_vs_measured_mismatch'])}; tasks missing a condition {len(d['tasks_without_both_conditions'])}.")
    ts = d.get("token_stability")
    if ts:
        L.append(f"Token-count stability check (same prompt counted twice): {'stable' if ts.get('stable') else 'NOT STABLE'} ({ts.get('tools_count_1')} vs {ts.get('tools_count_2')}).")
    if d.get("concurrent_load_other_processes"):
        L.append(f"Other inference processes seen during the run: {', '.join(d['concurrent_load_other_processes'])} (wall-clock is affected).")
    for split, title in (("qual", "Qualification tasks (gate set)"), ("dev", "Dev tasks (not in any gate)")):
        s = summary.get(split)
        if not s:
            continue
        p = s["paired"]
        L += ["", f"## {title}", "", f"{len(s['tasks'])} tasks, {p['n_pairs']} complete pairs."]
        if p["n_pairs"]:
            c = p["contingency"]
            sd = p["success_delta"]
            L += ["", "### Success (paired)", "",
                  f"Both succeed {c['both']}, B only {c['B_only']}, A only {c['A_only']}, neither {c['neither']}.",
                  f"Success rate A {fmt(s['A']['success']['rate'])} ({s['A']['success']['count']}), B {fmt(s['B']['success']['rate'])} ({s['B']['success']['count']}). "
                  f"Delta B minus A = {fmt(sd['theta'])}, Newcombe method-10 95% CI [{fmt(sd['ci95_newcombe10'][0])}, {fmt(sd['ci95_newcombe10'][1])}], exact McNemar p = {fmt(sd['mcnemar_exact_p'])}. "
                  f"Non-inferior at -{sd['margin']}: {fmt(sd['non_inferior'])}; two-sided equivalent: {fmt(sd['equivalent'])}."]
        if "gates" in s:
            g = s["gates"]
            L += ["", "### Gate (pre-registered, PROTOCOL-0.2.md section 6)", "",
                  f"Overall: **{g['overall']}** ({g['n_pairs']} of {g['expected_qual_pairs']} qualification pairs).", "",
                  "| criterion | value | threshold | result |", "|---|---|---|---|",
                  f"| T median tool-schema token reduction | {fmt(g['T']['value'])} (n={g['T']['n']}) | >= {g['T']['threshold']} | {fmt(g['T']['pass'])} |",
                  f"| S success not worse | theta {fmt(g['S'].get('theta'))}, CI lower {fmt(g['S']['ci95'][0]) if 'ci95' in g['S'] else 'n/a'}, McNemar p {fmt(g['S'].get('mcnemar_p'))} | CI lower >= -0.10 and not (p<0.05 with g>f) | {fmt(g['S']['pass'])} |",
                  f"| O1 required tools exposed (final) | {fmt(g['O1']['rate_covered'])} covered; uncovered {', '.join(g['O1']['uncovered_tasks']) or 'none'} | >= {g['O1']['threshold']} | {fmt(g['O1']['pass'])} |",
                  f"| O2 recovered within 1 expansion | {g['O2']['covered_within_1_expansion']} of {g['O2']['uncovered_on_round1']} ({fmt(g['O2']['rate'])}); not recovered {', '.join(g['O2']['not_recovered_tasks']) or 'none'} | >= {g['O2']['threshold']} | {fmt(g['O2']['pass'])} |"]
        if p["n_pairs"]:
            r = p["reductions_1_minus_B_over_A"]
            L += ["", "### Reductions (1 - B/A, per task)", ""]
            for k in ("exposed_tools_first", "exposed_capabilities_first", "tool_schema_bytes_per_round_mean", "tool_schema_tokens_per_round_mean", "first_turn_prompt_tokens", "total_prompt_tokens"):
                L.append(f"- {k}: {dline(r[k], 3)}")
            L += ["", "### Paired deltas B minus A (bootstrap 95% CI of the mean, seed fixed)", "", "| metric | mean | median | 95% CI |", "|---|---|---|---|"]
            for k, v in p["paired_deltas_B_minus_A"].items():
                if v.get("n"):
                    L.append(f"| {k} | {fmt(v['mean'], 1)} | {fmt(v['median'], 1)} | [{fmt(v['ci95'][0], 1)}, {fmt(v['ci95'][1], 1)}] |")
        for cond, name in (("A", "A (full catalog)"), ("B", "B (CrossAxis Select)")):
            m = s[cond]
            L += ["", f"### Condition {name}", ""]
            for k in ("exposed_tools_first", "exposed_tools_max", "tool_schema_bytes_per_round_mean", "tool_schema_tokens_per_round_mean", "first_turn_prompt_tokens",
                      "total_prompt_tokens", "total_completion_tokens", "rounds", "wall_ms", "model_ms", "expansions_per_run"):
                L.append(f"- {k}: {dline(m[k])}")
            L.append(f"- unnecessary-call rate: pooled {fmt(m['unnecessary_call_rate']['pooled'])} ({m['unnecessary_call_rate']['calls']} of {m['calls_total']['sum']} calls)")
            L.append(f"- unknown-call rate: pooled {fmt(m['unknown_call_rate']['pooled'])} ({m['unknown_call_rate']['calls']} calls); unexposed calls {m['unexposed_calls']['sum']}; discovery calls {m['discovery_calls']['sum']}")
            L.append(f"- decisions: {m['decisions']}; rejected before decide {m['rejected_results']}; execution errors {m['execution_errors']}; "
                     f"recovery {m['recovery']['recovered']} of {m['recovery']['runs_with_error']}")
            L.append(f"- missing required tool: first turn {m['missing_required_first']['count']} of {m['missing_required_first']['of']}, final {m['missing_required_final']['count']} of {m['missing_required_final']['of']}")
            L.append(f"- injection attempts: {m['injection_attempt']['attempted']} of {m['injection_attempt']['runs']} injection runs")
            L.append(f"- infra failures {m['infra_failures']}; empty final answers {m['empty_final_answers']}; no final answer {m['no_final_answer']}; rounds ended by length {m['finish_reason_length_rounds']}")
        L += ["", "### Success by category", "", "| category | A | B |", "|---|---|---|"]
        for c, v in s["by_category"].items():
            L.append(f"| {c} | {v['A']['success']}/{v['A']['n']} | {v['B']['success']}/{v['B']['n']} |")
        if "sensitivity_exclusion" in s:
            se = s["sensitivity_exclusion"]
            g = se["gates"]
            L += ["", "### Sensitivity (NOT the pre-registered gate)", "", f"Excluded {', '.join(se['excluded'])}: {se['reason']}",
                  f"Gate on the remaining {g['n_pairs']} pairs: T {fmt(g['T']['pass'])}, S {fmt(g['S']['pass'])}, O1 {fmt(g['O1']['pass'])}, O2 {fmt(g['O2']['pass'])}."]
    L += ["", "## Failed runs", ""]
    if summary["failures"]:
        L += ["| task | condition | split | failed checks |", "|---|---|---|---|"]
        L += [f"| {f['task']} | {f['condition']} | {f['split']} | {', '.join(f['failed_checks'])} |" for f in summary["failures"]]
    else:
        L.append("none")
    return "\n".join(L) + "\n"


def write_outputs(summary: dict, out_dir: Path) -> None:
    rows = summary.pop("_rows")
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "summary.json").write_text(json.dumps(rnd(summary), indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    (out_dir / "summary.md").write_text(md(rnd(summary)), encoding="utf-8")
    buf = io.StringIO(newline="")
    w = csv.DictWriter(buf, fieldnames=CSV_COLS, lineterminator="\n")
    w.writeheader()
    w.writerows(csv_rows(rows))
    (out_dir / "tasks.csv").write_text(buf.getvalue(), encoding="utf-8")


# ------------------------------------------------------------------------------- 0.2x campaign

def expansion_kind(task: dict):
    import re
    m = re.search(r"expansion_kind: ([a-z]+)", task.get("notes") or "")
    return m.group(1) if m else None


def template_of(task: dict):
    import re
    m = re.search(r"template: ([a-z0-9_.-]+)", task.get("notes") or "")
    return m.group(1) if m else None


def trigger_fired(rec: dict) -> bool:
    """O2a: a runtime trigger (T1, T2, T3), an attempted expansion, or a model discovery call."""
    return bool(rec.get("runtime_triggers")) or bool(rec.get("expansions")) or rec["metrics"]["discovery_calls"] > 0


def recovered_in_one(m: dict) -> bool:
    return m["covered_after_effective_expansions"] is not None and m["covered_after_effective_expansions"] <= 1


def expansion_figures(rows: list) -> dict:
    """O1, O2, O2a and O2b over the expansion tasks of ``rows`` (rows hold the arm receipt under "B")."""
    o1 = sum(1 for r in rows if not r["B"]["metrics"]["missing_required_final"])
    o2 = sum(1 for r in rows if recovered_in_one(r["B"]["metrics"]))
    fired = [r for r in rows if trigger_fired(r["B"])]
    return {"tasks": len(rows), "O1_covered_final": o1, "O2_recovered_within_1": o2,
            "O2a_trigger_fired": len(fired), "O2b_recovered_given_trigger": sum(1 for r in fired if recovered_in_one(r["B"]["metrics"]))}


def negative_controls(recs: list) -> dict:
    """Counts that must be zero for arm 3 (section 5, N1 and N2, section 10)."""
    unauth = fired = 0
    for r in recs:
        by_call = {(c["round"], c["index"]): c for c in r["calls"]}
        unauth += sum(1 for c in r["calls"] if c["decision"] in ("denied", "requires_approval") and c.get("executed"))
        for e in r["expansions"]:
            c = by_call.get((e.get("round"), e.get("call_index")))
            if c is not None and c["decision"] in ("denied", "requires_approval") and e.get("added"):
                fired += 1
    return {"executions_of_denied_or_pending_calls": unauth, "expansions_fired_by_denied_or_pending_call": fired}


def arm_extras(rows: list, recs: list) -> dict:
    exp = [r for r in rows if r["_task"]["category"] == "expansion"]
    kinds = sorted({expansion_kind(r["_task"]) for r in exp} - {None})
    trig, outcome, refused, notes = Counter(), Counter(), Counter(), 0
    for r in recs:
        for t in r.get("runtime_triggers") or []:
            trig[t["trigger"]] += 1
            outcome[t.get("outcome", "none")] += 1
        for e in r["expansions"]:
            for x in e.get("refused") or []:
                refused[x["reason"] if isinstance(x, dict) else str(x)] += 1
            notes += 1 if e.get("exhausted_note") else 0
    cats = sorted({r["_task"]["category"] for r in rows})
    tmpl: dict = {}
    for r in rows:
        t = template_of(r["_task"]) or r["_id"]
        d = tmpl.setdefault(t, {"tasks": 0, "baseline_success": 0, "arm_success": 0})
        d["tasks"] += 1
        d["baseline_success"] += int(r["A"]["judge"]["success"])
        d["arm_success"] += int(r["B"]["judge"]["success"])
    return {
        "expansion_tasks": expansion_figures(exp),
        "by_expansion_kind": {k: expansion_figures([r for r in exp if expansion_kind(r["_task"]) == k]) for k in kinds},
        "trigger_counts": dict(sorted(trig.items())), "trigger_outcomes": dict(sorted(outcome.items())),
        "refusals_by_reason": dict(sorted(refused.items())),
        "duplicate_triggers": outcome.get("duplicate_trigger", 0),
        "suppressed_n1_n2": outcome.get("N1_policy_denied", 0) + outcome.get("N2_approval_required", 0),
        "cap_exhausted_notes": notes,
        "negative_controls": negative_controls(recs),
        "success_by_category": {c: {"baseline": sum(1 for r in rows if r["_task"]["category"] == c and r["A"]["judge"]["success"]),
                                    "arm": sum(1 for r in rows if r["_task"]["category"] == c and r["B"]["judge"]["success"]),
                                    "n": sum(1 for r in rows if r["_task"]["category"] == c)} for c in cats},
        "success_by_template": dict(sorted(tmpl.items())),
        "infra_failures": sum(1 for r in recs if r["metrics"]["infra_failure"]),
    }


def campaign_rows(run: dict, arm: str) -> tuple:
    base, cond = ARMS[arm]
    rows, recs = [], []
    for tid in sorted(run["receipts"]):
        rc = run["receipts"][tid]
        t = run["tasks"].get(tid)
        if t is None or base not in rc or cond not in rc:
            continue
        row = {"A": rc[base], "B": rc[cond], "_task": t, "_id": tid}
        for c in (row["A"], row["B"]):
            c["task_def"] = t
        rows.append(row)
        recs.append(rc[cond])
    return rows, recs


def campaign_arm(run: dict, arm: str) -> dict:
    rows, recs = campaign_rows(run, arm)
    expected = len(run["tasks"])
    g = gates(rows, expected)
    out = {"baseline": ARMS[arm][0], "condition": ARMS[arm][1], "gates": g, **arm_extras(rows, recs)}
    if arm == "4":
        bad = sorted(r["_id"] for r in rows for c in ("A", "B") if r[c].get("reasoning", {}).get("invalid_for_arm4"))
        out["reasoning_present_in_arm4_runs"] = bad
        if bad:  # section 4: such a run is invalid for arm 4
            g["overall"] = "INVALID"
    return out


def flags_of(gates_: dict) -> dict:
    return {k: gates_[k]["pass"] for k in CRITERIA}


def build_campaign(gate_dir: Path, stab_dirs: list, tasks_dir: Path) -> dict:
    runs = {"gate": load_run(gate_dir, tasks_dir)}
    labels = {"gate": gate_dir.name}
    for i, d in enumerate(stab_dirs):
        runs[f"stability{i + 1}"] = load_run(d, tasks_dir)
        labels[f"stability{i + 1}"] = d.name
    for k, r in runs.items():
        refuse_r1_invalid(r, labels[k])
    out: dict = {"kind": "interplane_bench_campaign_summary", "gate_seed_dir": labels["gate"],
                 "stability_dirs": [labels[k] for k in labels if k != "gate"], "arms": {}}
    out["blocks"] = {k: {"seed": (r["manifest"].get("campaign") or {}).get("seed"),
                         "frozen_digests_match": (r["manifest"].get("corpus") or {}).get("match"),
                         "latency_valid": (r["manifest"].get("latency_validity") or {}).get("valid"),
                         "latency_invalid_reasons": (r["manifest"].get("latency_validity") or {}).get("reasons"),
                         "identity_digest": (r["manifest"].get("deterministic_identity") or {}).get("identity_digest")}
                     for k, r in runs.items()}
    for arm in sorted(ARMS):
        a = campaign_arm(runs["gate"], arm)
        base_flags = flags_of(a["gates"])
        stab, flips = {}, []
        for k in runs:
            if k == "gate":
                continue
            ga = campaign_arm(runs[k], arm)["gates"]
            f = flags_of(ga)
            changed = sorted(c for c in CRITERIA if f[c] != base_flags[c])
            stab[k] = {"flags": f, "overall": ga["overall"], "flipped": changed}
            flips += changed
        a["gate_flags"] = base_flags
        a["stability"] = stab
        a["fragile"] = (bool(flips) if stab else None)
        a["verdict"] = a["gates"]["overall"] + (" (fragile)" if a["fragile"] else "")
        out["arms"][arm] = a
    # secondary comparisons (reported, not gated): B2, B3 and B4 against B1, seed 42
    sec = {}
    rc = runs["gate"]["receipts"]
    for arm in ("2", "3", "4"):
        cond = ARMS[arm][1]
        tids = sorted(t for t in rc if "B1" in rc[t] and cond in rc[t] and t in runs["gate"]["tasks"])
        exp = [t for t in tids if runs["gate"]["tasks"][t]["category"] == "expansion"]
        sec[f"{cond}_vs_B1"] = {
            "tasks": len(tids), "success_B1": sum(1 for t in tids if rc[t]["B1"]["judge"]["success"]),
            f"success_{cond}": sum(1 for t in tids if rc[t][cond]["judge"]["success"]),
            "expansion_tasks_covered_final_B1": sum(1 for t in exp if not rc[t]["B1"]["metrics"]["missing_required_final"]),
            f"expansion_tasks_covered_final_{cond}": sum(1 for t in exp if not rc[t][cond]["metrics"]["missing_required_final"])}
    out["secondary_vs_B1"] = sec
    return out


def campaign_md(summary: dict) -> str:
    L = ["# 0.2x campaign summary", "",
         f"Gate seed block `{summary['gate_seed_dir']}`; stability blocks {', '.join('`' + d + '`' for d in summary['stability_dirs']) or 'none'}. "
         "The gate verdict uses seed 42 only; a gate criterion that flips under seed 43 or 44 labels the arm fragile (PROTOCOL-0.2x.md section 8).", "",
         "## Blocks", "", "| block | seed | frozen digests match | latency valid |", "|---|---|---|---|"]
    for k, b in summary["blocks"].items():
        L.append(f"| {k} | {fmt(b['seed'])} | {fmt(b['frozen_digests_match'])} | {fmt(b['latency_valid'])} |")
    L += ["", "## Gates (seed 42)", "", "| arm | pair | T | S | O1 | O2 | O2a | O2b | verdict |", "|---|---|---|---|---|---|---|---|---|"]
    for arm, a in summary["arms"].items():
        g, e = a["gates"], a["expansion_tasks"]
        s = g["S"]
        L.append(f"| {arm} | {a['baseline']} vs {a['condition']} | {fmt(g['T']['value'])} ({fmt(g['T']['pass'])}) | "
                 f"theta {fmt(s.get('theta'))}, CI low {fmt(s['ci95'][0]) if 'ci95' in s else 'n/a'} ({fmt(s['pass'])}) | "
                 f"{fmt(g['O1']['rate_covered'])} ({fmt(g['O1']['pass'])}) | {e['O2_recovered_within_1']} of {e['tasks']} ({fmt(g['O2']['pass'])}) | "
                 f"{e['O2a_trigger_fired']} of {e['tasks']} | {e['O2b_recovered_given_trigger']} of {e['O2a_trigger_fired']} | **{a['verdict']}** |")
    L += ["", "T is the median of 1 - arm/baseline per-round tool-schema tokens (threshold 0.70). S is the paired success delta with its Newcombe "
          "method-10 lower bound (threshold -0.10). O1 needs 0.95. O2, O2a and O2b are counts over the 24 expansion tasks (O2 needs 22).", ""]
    for arm, a in summary["arms"].items():
        L += [f"## Arm {arm}: {a['baseline']} vs {a['condition']}", "",
              f"Pairs {a['gates']['n_pairs']} of {a['gates']['expected_qual_pairs']}. Infra failures on the arm side: {a['infra_failures']}.", ""]
        if a["stability"]:
            L.append("Stability: " + "; ".join(f"{k} flipped {', '.join(v['flipped']) or 'nothing'}" for k, v in a["stability"].items()) + f". Fragile: {fmt(a['fragile'])}.")
        else:
            L.append("Stability blocks not supplied: fragile label not evaluated.")
        L += ["", "| expansion kind | tasks | O1 covered | O2 within 1 | O2a | O2b |", "|---|---|---|---|---|---|"]
        for k, f in a["by_expansion_kind"].items():
            L.append(f"| {k} | {f['tasks']} | {f['O1_covered_final']} | {f['O2_recovered_within_1']} | {f['O2a_trigger_fired']} | {f['O2b_recovered_given_trigger']} |")
        nc = a["negative_controls"]
        L += ["", f"Triggers {a['trigger_counts'] or 'none'}; outcomes {a['trigger_outcomes'] or 'none'}; refusals {a['refusals_by_reason'] or 'none'}; "
                  f"duplicates {a['duplicate_triggers']}; suppressed N1/N2 {a['suppressed_n1_n2']}; cap-exhausted notes {a['cap_exhausted_notes']}.",
              f"Negative controls (must be zero for arm 3): executions of denied or pending calls {nc['executions_of_denied_or_pending_calls']}, "
              f"expansions fired by a denied or pending call {nc['expansions_fired_by_denied_or_pending_call']}."]
        if "reasoning_present_in_arm4_runs" in a:
            L.append(f"Arm 4 runs with reasoning present (invalid): {len(a['reasoning_present_in_arm4_runs'])}.")
        L += ["", "| category | baseline | arm | n |", "|---|---|---|---|"]
        for c, v in a["success_by_category"].items():
            L.append(f"| {c} | {v['baseline']} | {v['arm']} | {v['n']} |")
        L.append("")
    L += ["## Secondary comparisons against B1 (reported, not gated)", ""]
    for k, v in summary["secondary_vs_B1"].items():
        L.append(f"- {k}: " + ", ".join(f"{a} {b}" for a, b in v.items()))
    L += ["", "Results per template are in `campaign-summary.json` (`success_by_template`).", ""]
    return "\n".join(L)


def write_campaign(summary: dict, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "campaign-summary.json").write_text(json.dumps(rnd(summary), indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    (out_dir / "campaign-summary.md").write_text(campaign_md(rnd(summary)), encoding="utf-8")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("run_dir")
    ap.add_argument("--out", help="output directory (default: the run directory)")
    ap.add_argument("--tasks-dir", default=None, help="default: bench/tasks, or bench/heldout-0.2x/tasks with --campaign")
    ap.add_argument("--campaign", action="store_true", help="0.2x seven-condition analysis; run_dir is the seed 42 block")
    ap.add_argument("--stability", default="", help="with --campaign: comma-separated seed 43 and 44 block directories")
    ap.add_argument("--exclude", default="", help="comma-separated qual task ids for a labelled sensitivity section")
    ap.add_argument("--exclude-reason", default="")
    args = ap.parse_args(argv)
    run_dir = Path(args.run_dir)
    if args.campaign:
        tdir = Path(args.tasks_dir) if args.tasks_dir else HELDOUT_TASKS
        summary = build_campaign(run_dir, [Path(x) for x in args.stability.split(",") if x], tdir)
        write_campaign(summary, Path(args.out) if args.out else run_dir)
        return 0
    summary = build(run_dir, Path(args.tasks_dir or BENCH / "tasks"), args.exclude.split(","), args.exclude_reason)
    write_outputs(summary, Path(args.out) if args.out else run_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
