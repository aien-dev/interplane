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

def load_run(run_dir: Path, tasks_dir: Path) -> dict:
    manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8")) if (run_dir / "manifest.json").exists() else {}
    rec = {}
    for p in sorted((run_dir / "receipts").glob("*.json")):
        r = json.loads(p.read_text(encoding="utf-8"))
        rec.setdefault(r["task"]["id"], {})[r["condition"]] = r
    tasks = {}
    for p in sorted(tasks_dir.glob("*.json")):
        t = json.loads(p.read_text(encoding="utf-8"))
        tasks[t["id"]] = t
    return {"manifest": manifest, "receipts": rec, "tasks": tasks}


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


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("run_dir")
    ap.add_argument("--out", help="output directory (default: the run directory)")
    ap.add_argument("--tasks-dir", default=str(BENCH / "tasks"))
    ap.add_argument("--exclude", default="", help="comma-separated qual task ids for a labelled sensitivity section")
    ap.add_argument("--exclude-reason", default="")
    args = ap.parse_args(argv)
    run_dir = Path(args.run_dir)
    summary = build(run_dir, Path(args.tasks_dir), args.exclude.split(","), args.exclude_reason)
    write_outputs(summary, Path(args.out) if args.out else run_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
