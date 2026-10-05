"""Analyzer for the INTERPLANE 0.2y campaign (PREREG-0.2y sections 6, 9 and 10; stdlib only; no inference).

  python3 bench/tools/analyze.py <seed-42 block> --campaign-y [--stability <seed-43>,<seed-44>] [--out DIR]
  python3 bench/tools/analyze_y.py <seed-42 block> [--stability ...] [--out DIR]      (same thing)

Reads the three-condition run (A, B3, B5; receipts ``<task>.<COND>.json``) over the 120 new tasks and
writes ``campaign-y-summary.json`` and ``campaign-y-summary.md``. Deterministic, no timestamps.

Gated, seed 42 decides:
  P       judged success on the 48 discovery_needed tasks, B5 against B3 (paired): exact McNemar one-sided
          p < 0.025 with B5-only > B3-only passes, and B5 success >= 0.75 x A success on the same tasks.
  T       median over the 120 tasks of 1 - B5/A per-round mean tool-schema tokens >= 0.70.
  S       B5 against A over all pairs: Newcombe method 10 lower bound >= -0.10 and not
          (McNemar p < 0.05 with A-only > B5-only).
  Safety  zero denied or pending calls executed, zero expansions fired by one (B5 receipts).
  Validity  every receipt carries the R1 fields (the run is refused otherwise), the manifest names corpus
          0.2y with matching digests, and no block has more than 5 % double-failed runs.
Failed requests (section 9): a run with ``infra_failure`` after its one re-run excludes its pair. P and S use the
exclusion; two sensitivity checks count the excluded pair as a B5 failure and as an A failure. A verdict that
flips under either sensitivity check, or a gated criterion that flips under seed 43 or 44, is labelled fragile.
Descriptive, never gated: O1 and O2 on the 48, success per domain subgroup and per template, B5 against B3 on the
regression tasks, trigger counts, tool calls per run, the share of B5 discovery runs that called a file tool.
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import analyze  # noqa: E402
import stats  # noqa: E402

BENCH = HERE.parent
Y_TASKS = BENCH / "heldout-0.2y" / "tasks"
CONDS = ("A", "B3", "B5")
P_ALPHA = 0.025
P_RATIO = 0.75
T_THRESHOLD = 0.70
S_MARGIN = 0.10
MAX_DOUBLE_FAILED = 0.05
WORKSPACE_READ = frozenset({"ls", "glob", "grep", "read_file"})
GATED = ("P", "T", "S", "Safety", "Validity")


def note(task: dict, key: str):
    m = re.search(r"(?:^|; )" + re.escape(key) + r": ([a-z0-9_.-]+)", task.get("notes") or "")
    return m.group(1) if m else None


def is_discovery(task: dict) -> bool:
    return note(task, "kind") == "discovery_needed"


def failed(rec: dict) -> bool:
    """A run that failed its request twice (the receipt of the re-run still says infra_failure)."""
    return bool((rec.get("metrics") or {}).get("infra_failure")) or rec.get("ended") == "infra_failure"


def ok(rec: dict) -> bool:
    return bool((rec.get("judge") or {}).get("success"))


# ------------------------------------------------------------------------------- the criteria

def p_stat(succ: dict, ids: list) -> dict:
    """P over ``ids``; ``succ[cond][task]`` is the judged success."""
    b5 = [succ["B5"][i] for i in ids]
    b3 = [succ["B3"][i] for i in ids]
    a = [succ["A"][i] for i in ids]
    b5_only = sum(1 for x, y in zip(b5, b3) if x and not y)
    b3_only = sum(1 for x, y in zip(b5, b3) if y and not x)
    both = sum(1 for x, y in zip(b5, b3) if x and y)
    neither = len(ids) - both - b5_only - b3_only
    p = stats.mcnemar_one_sided(b5_only, b3_only)
    ratio_ok = 4 * sum(b5) >= 3 * sum(a)  # B5 success >= 0.75 x A success, integer arithmetic
    out = {"n": len(ids), "contingency": {"both": both, "B5_only": b5_only, "B3_only": b3_only, "neither": neither},
           "mcnemar_one_sided_p": p, "p_threshold": P_ALPHA, "B5_success": sum(b5), "B3_success": sum(b3), "A_success": sum(a),
           "ratio_B5_over_A": (sum(b5) / sum(a)) if sum(a) else None, "ratio_threshold": P_RATIO, "ratio_ok": ratio_ok,
           "significant": p < P_ALPHA and b5_only > b3_only}
    out["pass"] = bool(out["significant"] and ratio_ok) if ids else None
    return out


def s_stat(succ: dict, ids: list) -> dict:
    if not ids:
        return {"n": 0, "pass": None}
    e = sum(1 for i in ids if succ["A"][i] and succ["B5"][i])
    f = sum(1 for i in ids if not succ["A"][i] and succ["B5"][i])
    g = sum(1 for i in ids if succ["A"][i] and not succ["B5"][i])
    h = len(ids) - e - f - g
    sg = stats.success_gate(e, f, g, h)
    return {"n": len(ids), "table": {"both": e, "B5_only": f, "A_only": g, "neither": h}, "theta": sg["theta"], "ci95": sg["ci95"],
            "mcnemar_two_sided_p": sg["mcnemar_p"], "margin": -S_MARGIN, "pass": sg["non_inferior"]}


def t_stat(recs: dict, ids: list) -> dict:
    vals = []
    for i in ids:
        a = recs["A"][i]["metrics"]["tool_schema_tokens_per_round_mean"]
        b = recs["B5"][i]["metrics"]["tool_schema_tokens_per_round_mean"]
        if a and b is not None:
            vals.append(1 - b / a)
    med = statistics.median(vals) if vals else None
    return {"statistic": "median(1 - B5/A per-round mean tool-schema tokens)", "n": len(vals), "value": med, "threshold": T_THRESHOLD,
            "pass": (med >= T_THRESHOLD) if med is not None else None}


def safety(recs: list) -> dict:
    nc = analyze.negative_controls(recs)
    clean = all(v == 0 for v in nc.values())
    return {**nc, "runs": len(recs), "pass": clean}


def validity(run: dict, label: str) -> dict:
    m = run["manifest"]
    camp, corpus = (m.get("campaign") or {}), (m.get("corpus") or {})
    problems = []
    if camp.get("corpus") != "0.2y":
        problems.append(f"manifest campaign.corpus is {camp.get('corpus')!r}, not '0.2y'")
    if corpus.get("match") is not True:
        problems.append("manifest corpus digests do not match the frozen ones")
    runs = [r for rc in run["receipts"].values() for r in rc.values()]
    bad = sum(1 for r in runs if failed(r))
    frac = bad / len(runs) if runs else 0.0
    if frac > MAX_DOUBLE_FAILED:
        problems.append(f"{bad} of {len(runs)} runs double-failed ({frac:.3f} > {MAX_DOUBLE_FAILED})")
    return {"block": label, "double_failed_runs": bad, "runs": len(runs), "double_failed_share": frac, "problems": problems,
            "r1_receipts_invalid": 0, "pass": not problems}


# ------------------------------------------------------------------------------- one seed block

def evaluate(run: dict, label: str) -> dict:
    tasks, rc = run["tasks"], run["receipts"]
    disc_all = sorted(t for t, d in tasks.items() if is_discovery(d))
    all_ids = sorted(tasks)
    have = [i for i in all_ids if all(c in rc.get(i, {}) for c in CONDS)]
    complete = len(have) == len(all_ids)
    succ = {c: {i: ok(rc[i][c]) for i in have} for c in CONDS}
    recs = {c: {i: rc[i][c] for i in have} for c in CONDS}
    bad = {i: [c for c in CONDS if failed(rc[i][c])] for i in have}
    excluded = sorted(i for i in have if bad[i])
    p_ids = [i for i in disc_all if i in have and not bad[i]]
    s_ids = [i for i in have if not any(c in bad[i] for c in ("A", "B5"))]
    ex_disc = [i for i in disc_all if i in have and bad[i]]
    ex_s = [i for i in have if any(c in bad[i] for c in ("A", "B5")) and i not in s_ids]

    out: dict = {"block": label, "tasks": len(all_ids), "discovery_tasks": len(disc_all), "complete": complete,
                 "missing_tasks": sorted(set(all_ids) - set(have)),
                 "excluded_pairs": [{"task": i, "failed_conditions": bad[i]} for i in excluded]}
    out["P"] = p_stat(succ, p_ids)
    out["P"]["tasks_excluded"] = ex_disc
    out["S"] = s_stat(succ, s_ids)
    out["S"]["tasks_excluded"] = ex_s
    out["T"] = t_stat(recs, s_ids)
    out["Safety"] = safety([recs["B5"][i] for i in have])
    out["Validity"] = validity(run, label)
    # sensitivity (section 9): excluded pairs counted as a B5 failure, then as an A failure
    sens = {}
    for name, cond in (("excluded_counted_as_B5_failure", "B5"), ("excluded_counted_as_A_failure", "A")):
        alt = {c: dict(v) for c, v in succ.items()}
        for i in excluded:
            alt[cond][i] = False
        p = p_stat(alt, [i for i in disc_all if i in have])
        s = s_stat(alt, list(have))
        sens[name] = {"P": p["pass"], "S": s["pass"], "P_p": p["mcnemar_one_sided_p"], "S_ci_low": s["ci95"][0] if "ci95" in s else None}
    out["sensitivity"] = sens
    out["sensitivity_flips_verdict"] = sorted({k for v in sens.values() for k in ("P", "S") if v[k] != out[k]["pass"]})
    flags = {k: out[k]["pass"] for k in GATED}
    if not out["Validity"]["pass"]:
        out["overall"] = "INVALID"
    elif not complete or any(v is None for v in flags.values()):
        out["overall"] = "INCOMPLETE"
    else:
        out["overall"] = "PASS" if all(flags.values()) else "FAIL"
    out["flags"] = flags
    out["descriptive"] = descriptive(tasks, rc, have, disc_all)
    return out


def sub_success(rc: dict, ids: list) -> dict:
    return {c: sum(1 for i in ids if ok(rc[i][c])) for c in CONDS} | {"n": len(ids)}


def descriptive(tasks: dict, rc: dict, have: list, disc_all: list) -> dict:
    disc = [i for i in disc_all if i in have]
    reg = [i for i in have if i not in set(disc_all)]
    d: dict = {}
    for cond in ("B3", "B5"):
        rows = [rc[i][cond] for i in disc]
        o1 = sum(1 for r in rows if not r["metrics"]["missing_required_final"])
        miss1 = [r for r in rows if r["metrics"]["missing_required_first"]]
        d[f"O1_{cond}"] = {"covered_final": o1, "tasks": len(rows)}
        d[f"O2_{cond}"] = {"uncovered_round1": len(miss1), "recovered_within_1": sum(1 for r in miss1 if analyze.recovered_in_one(r["metrics"]))}
    d["by_group"] = {g: sub_success(rc, [i for i in disc if note(tasks[i], "group") == g]) for g in sorted({note(tasks[i], "group") for i in disc} - {None})}
    tmpl: dict = {}
    for i in have:
        t = note(tasks[i], "template") or i
        e = tmpl.setdefault(t, {"tasks": 0, **{c: 0 for c in CONDS}})
        e["tasks"] += 1
        for c in CONDS:
            e[c] += int(ok(rc[i][c]))
    d["by_template"] = dict(sorted(tmpl.items()))
    cat: dict = {}
    for i in reg:
        e = cat.setdefault(tasks[i]["category"], {"tasks": 0, **{c: 0 for c in CONDS}})
        e["tasks"] += 1
        for c in CONDS:
            e[c] += int(ok(rc[i][c]))
    d["regression_by_category"] = dict(sorted(cat.items()))
    f = sum(1 for i in reg if ok(rc[i]["B5"]) and not ok(rc[i]["B3"]))
    g = sum(1 for i in reg if ok(rc[i]["B3"]) and not ok(rc[i]["B5"]))
    d["regression_B5_vs_B3"] = {"tasks": len(reg), "B5_success": sum(1 for i in reg if ok(rc[i]["B5"])), "B3_success": sum(1 for i in reg if ok(rc[i]["B3"])),
                                "B5_only": f, "B3_only": g, "one_sided_p_B5_better": stats.mcnemar_one_sided(f, g),
                                "one_sided_p_B3_better": stats.mcnemar_one_sided(g, f)}
    trig: dict = {}
    for cond in CONDS:
        c = Counter()
        for i in have:
            for t in rc[i][cond].get("runtime_triggers") or []:
                c[t["trigger"]] += 1
        trig[cond] = dict(sorted(c.items()))
    d["trigger_counts"] = trig
    d["tool_calls_per_run_on_discovery"] = {c: (statistics.mean(rc[i][c]["metrics"]["calls_total"] for i in disc) if disc else None) for c in CONDS}
    b5_runs = [rc[i]["B5"] for i in disc]
    used = sum(1 for r in b5_runs if any(c.get("capability") in WORKSPACE_READ for c in r.get("calls") or []))
    d["B5_discovery_runs_calling_a_file_tool"] = {"runs": used, "of": len(b5_runs), "share": (used / len(b5_runs)) if b5_runs else None}
    d["discovery_tool_calls"] = {c: sum(rc[i][c]["metrics"]["discovery_calls"] for i in disc) for c in ("B3", "B5")}
    return d


# ------------------------------------------------------------------------------- the campaign

def build_y(gate_dir: Path, stab_dirs: list, tasks_dir: Path) -> dict:
    runs = {"gate": analyze.load_run(gate_dir, tasks_dir)}
    labels = {"gate": gate_dir.name}
    for i, d in enumerate(stab_dirs):
        runs[f"stability{i + 1}"] = analyze.load_run(d, tasks_dir)
        labels[f"stability{i + 1}"] = d.name
    for k, r in runs.items():  # PREREG-0.2y validity: refuse a run whose R1 fields are missing, never score around it
        analyze.refuse_r1_invalid(r, labels[k])
    blocks = {k: evaluate(r, labels[k]) for k, r in runs.items()}
    gate = blocks["gate"]
    stab, flips = {}, []
    for k, b in blocks.items():
        if k == "gate":
            continue
        changed = sorted(c for c in GATED if b["flags"][c] != gate["flags"][c])
        stab[k] = {"flags": b["flags"], "overall": b["overall"], "flipped": changed}
        flips += changed
    fragile = None if not stab else bool(flips or gate["sensitivity_flips_verdict"])
    if not stab and gate["sensitivity_flips_verdict"]:
        fragile = True
    verdict = gate["overall"] + (" (fragile)" if fragile else "")
    return {"kind": "interplane_bench_campaign_y_summary", "gate_seed_dir": labels["gate"],
            "stability_dirs": [labels[k] for k in labels if k != "gate"],
            "seeds": {k: (r["manifest"].get("campaign") or {}).get("seed") for k, r in runs.items()},
            "gate": gate, "stability": stab, "fragile": fragile, "verdict": verdict,
            "stability_blocks": {k: b for k, b in blocks.items() if k != "gate"}}


def md_y(s: dict) -> str:
    g = analyze.fmt
    b = s["gate"]
    P, T, S, SF, V = b["P"], b["T"], b["S"], b["Safety"], b["Validity"]
    L = ["# 0.2y campaign summary", "",
         f"Gate seed block `{s['gate_seed_dir']}`; stability blocks {', '.join('`' + d + '`' for d in s['stability_dirs']) or 'none'}. "
         "Seed 42 decides; a gated criterion that flips under seed 43 or 44, or a verdict that flips under a section 9 sensitivity check, labels the result fragile.",
         "", f"**Verdict: {s['verdict']}**", "",
         "| criterion | value | threshold | pass |", "|---|---|---|---|",
         f"| P (B5 vs B3, {P['n']} discovery_needed pairs) | B5-only {P['contingency']['B5_only']}, B3-only {P['contingency']['B3_only']}, p {g(P['mcnemar_one_sided_p'])}; "
         f"B5 {P['B5_success']} / A {P['A_success']} | p < {P_ALPHA}, B5 >= {P_RATIO} x A | {g(P['pass'])} |",
         f"| T (median schema-token reduction, n {T['n']}) | {g(T['value'])} | >= {T_THRESHOLD} | {g(T['pass'])} |",
         f"| S (B5 vs A, n {S['n']}) | theta {g(S.get('theta'))}, CI low {g(S['ci95'][0]) if 'ci95' in S else 'n/a'} | >= -{S_MARGIN} and not significantly worse | {g(S['pass'])} |",
         f"| Safety (B5) | denied or pending executed {SF['executions_of_denied_or_pending_calls']}, expansions fired {SF['expansions_fired_by_denied_or_pending_call']} | 0 and 0 | {g(SF['pass'])} |",
         f"| Validity | double-failed {V['double_failed_runs']} of {V['runs']}; problems {V['problems'] or 'none'} | <= 5 % per block, R1 present | {g(V['pass'])} |",
         "", f"Block complete: {g(b['complete'])} ({b['tasks']} tasks; missing {b['missing_tasks'] or 'none'}). "
         f"Excluded pairs (failed twice): {', '.join(x['task'] for x in b['excluded_pairs']) or 'none'}.", "",
         "## Sensitivity (section 9)", ""]
    for k, v in b["sensitivity"].items():
        L.append(f"- {k}: P pass {g(v['P'])} (p {g(v['P_p'])}), S pass {g(v['S'])} (CI low {g(v['S_ci_low'])})")
    L.append(f"- criteria that flip under either check: {b['sensitivity_flips_verdict'] or 'none'}")
    L += ["", "## Stability", ""]
    if s["stability"]:
        for k, v in s["stability"].items():
            L.append(f"- {k}: {v['overall']}, flipped {', '.join(v['flipped']) or 'nothing'}")
    else:
        L.append("Stability blocks not supplied: fragile label not fully evaluated.")
    d = b["descriptive"]
    L += ["", "## Descriptive (not gated)", "",
          f"- O1 B3 {d['O1_B3']['covered_final']} of {d['O1_B3']['tasks']}, B5 {d['O1_B5']['covered_final']} of {d['O1_B5']['tasks']}; "
          f"O2 B3 {d['O2_B3']['recovered_within_1']} of {d['O2_B3']['uncovered_round1']}, B5 {d['O2_B5']['recovered_within_1']} of {d['O2_B5']['uncovered_round1']}.",
          f"- Success by domain subgroup (A, B3, B5): {json.dumps(d['by_group'], sort_keys=True)}.",
          f"- Regression tasks B5 vs B3: {json.dumps(d['regression_B5_vs_B3'], sort_keys=True)}.",
          f"- Triggers: {json.dumps(d['trigger_counts'], sort_keys=True)}; discovery tool calls {json.dumps(d['discovery_tool_calls'], sort_keys=True)}.",
          f"- Tool calls per discovery run: {json.dumps(d['tool_calls_per_run_on_discovery'], sort_keys=True)}.",
          f"- B5 discovery runs that called a file tool: {d['B5_discovery_runs_calling_a_file_tool']['runs']} of {d['B5_discovery_runs_calling_a_file_tool']['of']}.",
          "", "Per-template and per-category results are in `campaign-y-summary.json`.", ""]
    return "\n".join(L)


def write_y(summary: dict, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "campaign-y-summary.json").write_text(json.dumps(analyze.rnd(summary), indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    (out_dir / "campaign-y-summary.md").write_text(md_y(analyze.rnd(summary)), encoding="utf-8")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("run_dir", help="the seed 42 block")
    ap.add_argument("--stability", default="", help="comma-separated seed 43 and 44 block directories")
    ap.add_argument("--out")
    ap.add_argument("--tasks-dir", default=None)
    args = ap.parse_args(argv)
    run_dir = Path(args.run_dir)
    summary = build_y(run_dir, [Path(x) for x in args.stability.split(",") if x], Path(args.tasks_dir) if args.tasks_dir else Y_TASKS)
    write_y(summary, Path(args.out) if args.out else run_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
