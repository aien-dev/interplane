"""Offline tests of the 0.2y analyzer on synthetic receipts (stdlib only; no model, no GPU).

  python3 bench/tools/test_analyze_y.py

Synthetic tasks and receipts carry only the fields the analyzer reads. Scenarios: a PASS, each gated
criterion failing alone, the exact P boundary (6 to 0 passes, 7 to 1 does not), the failed-request exclusion
with its two sensitivity checks, an invalid block, missing R1 fields, an incomplete block, seed stability, and
determinism of the written outputs.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import analyze  # noqa: E402
import analyze_y as Y  # noqa: E402
import stats  # noqa: E402

N_DISC, N_REG = 48, 12
DIGEST = "sha256:" + "ab" * 32
analyze.DIGEST_0_2Y = Path("/nonexistent/CORPUS-DIGEST.txt")  # the frozen file does not exist yet; tests must not depend on it


def tasks():
    t = {}
    for i in range(N_DISC):
        g = "nonfile" if i < 24 else "default"
        t[f"ambiguous-{100 + i}"] = {"id": f"ambiguous-{100 + i}", "category": "ambiguous", "split": "qual",
                                    "notes": f"heldout-0.2y; template: d{i}; kind: discovery_needed; group: {g}; request: q-{i + 1:04d}"}
    for i in range(N_REG):
        t[f"filesystem-{i + 1:03d}"] = {"id": f"filesystem-{i + 1:03d}", "category": "filesystem", "split": "qual",
                                        "notes": f"heldout-0.2y; template: r{i}; kind: regression"}
    return t


def receipt(task, cond, success, seed=42, tokens=None, infra=False, denied_executed=False, file_tool=True, with_text=True):
    tok = tokens if tokens is not None else (9000.0 if cond == "A" else 900.0)
    calls = [{"round": 1, "index": 0, "tool": "read_file", "capability": "read_file" if file_tool else "x", "decision": "authorized",
              "status": "ok", "executed": True, "discovery": False}]
    if denied_executed:
        calls.append({"round": 1, "index": 1, "tool": "manage_tokens", "capability": "manage_tokens", "decision": "denied",
                      "status": "ok", "executed": True, "discovery": False})
    rounds = [{"round": 1, "exposed_caps_count": 5}]
    if with_text:
        rounds[0]["text"] = "answer"
    return {"kind": "interplane_bench_receipt", "task": {"id": task["id"], "category": task["category"], "split": "qual"}, "condition": cond,
            "attempt": 2 if infra else 1, "rounds": rounds, "calls": calls, "expansions": [], "ended": "infra_failure" if infra else "answer",
            "judge": {"success": bool(success) and not infra}, "generation": {"seed": seed},
            "metrics": {"infra_failure": infra, "tool_schema_tokens_per_round_mean": None if infra else tok,
                        "missing_required_final": False, "missing_required_first": cond != "A" and False,
                        "covered_after_effective_expansions": 0, "calls_total": len(calls), "discovery_calls": 0}}


def write_block(root: Path, name: str, ts: dict, outcome, seed=42, **kw):
    """outcome(task_id, cond) -> bool or a dict of receipt overrides."""
    d = root / name
    (d / "receipts").mkdir(parents=True)
    (d / "manifest.json").write_text(json.dumps({"campaign": {"corpus": "0.2y", "seed": seed},
                                                 "corpus": {"frozen": {"tasks_digest": DIGEST}, "recomputed": {"tasks_digest": DIGEST}, "match": True},
                                                 "latency_validity": {"valid": True}}))
    for tid, t in ts.items():
        for cond in Y.CONDS:
            o = outcome(tid, cond)
            args = {"success": o} if isinstance(o, bool) else dict(o)
            rec = receipt(t, cond, seed=seed, **{**kw, **args})
            (d / "receipts" / f"{tid}.{cond}.json").write_text(json.dumps(rec))
    return d


def tdir(root: Path, ts: dict) -> Path:
    d = root / "tasks"
    d.mkdir()
    for tid, t in ts.items():
        (d / f"{tid}.json").write_text(json.dumps(t))
    return d


def ids(prefix_disc=True):
    return [f"ambiguous-{100 + i}" for i in range(N_DISC)]


def standard(tid, cond, a_pass=24, b5_extra=0, b3_pass=0):
    """A passes the first ``a_pass`` discovery tasks, B3 the first ``b3_pass``, B5 the first ``a_pass`` plus nothing else.
    Regression tasks: every condition passes."""
    if tid.startswith("filesystem"):
        return True
    k = int(tid.split("-")[1]) - 100
    if cond == "A":
        return k < a_pass
    if cond == "B3":
        return k < b3_pass
    return k < a_pass + b5_extra


class Gates(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.ts = tasks()
        self.tdir = tdir(self.root, self.ts)

    def tearDown(self):
        self._tmp.cleanup()

    def run_y(self, outcome, stability=(), **kw):
        gate = write_block(self.root, "seed-42", self.ts, outcome, 42, **kw)
        stab = [write_block(self.root, f"seed-{s}", self.ts, o, s) for s, o in stability]
        return Y.build_y(gate, stab, self.tdir)

    def test_pass(self):
        s = self.run_y(standard)
        g = s["gate"]
        self.assertEqual(s["verdict"], "PASS", json.dumps(g["flags"]))
        self.assertEqual(g["P"]["contingency"], {"both": 0, "B5_only": 24, "B3_only": 0, "neither": 24})
        self.assertTrue(g["P"]["pass"] and g["T"]["pass"] and g["S"]["pass"] and g["Safety"]["pass"] and g["Validity"]["pass"])
        self.assertAlmostEqual(g["T"]["value"], 0.9)
        self.assertEqual(g["P"]["n"], 48)
        self.assertEqual(g["descriptive"]["B5_discovery_runs_calling_a_file_tool"]["runs"], 48)

    def test_p_fails_when_b5_does_not_beat_b3(self):
        s = self.run_y(lambda t, c: standard(t, c, b3_pass=24))
        g = s["gate"]
        self.assertEqual(g["P"]["contingency"]["B5_only"], 0)
        self.assertFalse(g["P"]["pass"])
        self.assertEqual(s["verdict"], "FAIL")
        self.assertTrue(g["T"]["pass"] and g["S"]["pass"])

    def test_p_fails_on_the_ratio_even_when_significant(self):
        # A passes 40 of 48, B3 none, B5 only 7: p = 1/128 < 0.025 but 7 < 0.75 x 40
        def o(t, c):
            if t.startswith("filesystem"):
                return True
            k = int(t.split("-")[1]) - 100
            return {"A": k < 40, "B3": False, "B5": k < 7}[c]
        g = self.run_y(o)["gate"]
        self.assertTrue(g["P"]["significant"])
        self.assertFalse(g["P"]["ratio_ok"])
        self.assertFalse(g["P"]["pass"])

    def test_exact_p_boundary(self):
        self.assertAlmostEqual(stats.mcnemar_one_sided(6, 0), 0.015625)
        self.assertAlmostEqual(stats.mcnemar_one_sided(7, 1), 0.03515625)
        win = {"A": {i: True for i in range(6)}, "B3": {i: False for i in range(6)}, "B5": {i: True for i in range(6)}}
        r = Y.p_stat(win, list(range(6)))
        self.assertTrue(r["pass"], r)
        loss = {"A": {i: True for i in range(8)}, "B3": {i: i == 7 for i in range(8)}, "B5": {i: i < 7 for i in range(8)}}
        r = Y.p_stat(loss, list(range(8)))
        self.assertEqual(r["contingency"]["B5_only"] - r["contingency"]["B3_only"], 6)
        self.assertEqual((r["contingency"]["B5_only"], r["contingency"]["B3_only"]), (7, 1))
        self.assertFalse(r["pass"], r)  # 7 to 1: p = 0.035, not enough

    def test_t_fails_on_schema_cost(self):
        self.assertTrue(self.run_y(standard)["gate"]["T"]["pass"])
        gate = write_block(self.root, "seed-heavy", self.ts, standard, 42)
        for p in (gate / "receipts").glob("*.B5.json"):
            r = json.loads(p.read_text())
            r["metrics"]["tool_schema_tokens_per_round_mean"] = 4000.0
            p.write_text(json.dumps(r))
        g = Y.build_y(gate, [], self.tdir)["gate"]
        self.assertFalse(g["T"]["pass"])
        self.assertEqual(g["overall"], "FAIL")

    def test_s_fails_when_b5_loses_to_a_on_regressions(self):
        def o(t, c):
            if t.startswith("filesystem"):
                return not (c == "B5" and int(t.split("-")[1]) <= 8)
            return standard(t, c)
        g = self.run_y(o)["gate"]
        self.assertFalse(g["S"]["pass"])
        self.assertTrue(g["P"]["pass"])

    def test_safety_fails_on_an_executed_denied_call(self):
        def o(t, c):
            r = standard(t, c)
            if c == "B5" and t == "ambiguous-100":
                return {"success": r, "denied_executed": True}
            return r
        g = self.run_y(o)["gate"]
        self.assertEqual(g["Safety"]["executions_of_denied_or_pending_calls"], 1)
        self.assertFalse(g["Safety"]["pass"])
        self.assertEqual(g["overall"], "FAIL")

    def test_failed_pair_is_excluded_and_listed_and_sensitivity_can_flip(self):
        # 6 B5-only wins, 0 B3-only: P passes. One extra task where B3 passes but B5 request failed twice.
        def o(t, c):
            if t.startswith("filesystem"):
                return True
            k = int(t.split("-")[1]) - 100
            if k == 47:
                return {"success": c != "B5", **({"infra": True} if c == "B5" else {})}
            if c == "A":
                return k < 6 or k == 47
            if c == "B3":
                return k == 47
            return k < 6
        s = self.run_y(o)
        g = s["gate"]
        self.assertEqual([x["task"] for x in g["excluded_pairs"]], ["ambiguous-147"])
        self.assertEqual(g["P"]["tasks_excluded"], ["ambiguous-147"])
        self.assertEqual(g["P"]["n"], 47)
        self.assertEqual((g["P"]["contingency"]["B5_only"], g["P"]["contingency"]["B3_only"]), (6, 0))
        self.assertTrue(g["P"]["pass"])
        # counted as a B5 failure the pair becomes B3-only: 6 to 1, p = 0.0625: P flips
        self.assertFalse(g["sensitivity"]["excluded_counted_as_B5_failure"]["P"])
        self.assertIn("P", g["sensitivity_flips_verdict"])
        self.assertTrue(s["fragile"])
        self.assertTrue(s["verdict"].endswith("(fragile)"), s["verdict"])

    def test_more_than_five_percent_double_failed_makes_the_block_invalid(self):
        n = [0]

        def o(t, c):
            n[0] += 1
            r = standard(t, c)
            return {"success": r, "infra": True} if n[0] % 10 == 0 else r
        s = self.run_y(o)
        self.assertEqual(s["gate"]["overall"], "INVALID")
        self.assertFalse(s["gate"]["Validity"]["pass"])
        self.assertTrue(any("double-failed" in p for p in s["gate"]["Validity"]["problems"]))

    def test_missing_r1_text_refuses_the_run(self):
        with self.assertRaises(SystemExit) as cm:
            self.run_y(lambda t, c: standard(t, c) if c != "B5" else {"success": True, "with_text": False})
        self.assertIn("R1", str(cm.exception))

    def test_wrong_manifest_identity_is_invalid(self):
        gate = write_block(self.root, "seed-42", self.ts, standard, 42)
        m = json.loads((gate / "manifest.json").read_text())
        m["corpus"]["match"] = False
        (gate / "manifest.json").write_text(json.dumps(m))
        g = Y.build_y(gate, [], self.tdir)["gate"]
        self.assertEqual(g["overall"], "INVALID")
        self.assertTrue(any("digests do not match" in p for p in g["Validity"]["problems"]))

    def test_incomplete_block(self):
        gate = write_block(self.root, "seed-42", self.ts, standard, 42)
        (gate / "receipts" / "ambiguous-100.B5.json").unlink()
        g = Y.build_y(gate, [], self.tdir)["gate"]
        self.assertEqual(g["overall"], "INCOMPLETE")
        self.assertEqual(g["missing_tasks"], ["ambiguous-100"])

    def test_stability_flip_marks_fragile(self):
        weak = lambda t, c: standard(t, c, b3_pass=24)  # noqa: E731  (B5 no better than B3 in seed 43)
        s = self.run_y(standard, stability=[(43, standard), (44, weak)])
        self.assertEqual(s["gate"]["overall"], "PASS")
        self.assertEqual(s["stability"]["stability2"]["flipped"], ["P"])
        self.assertTrue(s["fragile"])
        self.assertEqual(s["verdict"], "PASS (fragile)")

    def test_stable_pass_is_not_fragile(self):
        s = self.run_y(standard, stability=[(43, standard), (44, standard)])
        self.assertFalse(s["fragile"])
        self.assertEqual(s["verdict"], "PASS")

    def test_cli_is_deterministic_and_writes_both_files(self):
        gate = write_block(self.root, "seed-42", self.ts, standard, 42)
        outs = []
        for n in (1, 2):
            out = self.root / f"out{n}"
            r = subprocess.run([sys.executable, str(HERE / "analyze.py"), str(gate), "--campaign-y", "--tasks-dir", str(self.tdir), "--out", str(out)],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            outs.append({p.name: p.read_text() for p in out.iterdir()})
        self.assertEqual(sorted(outs[0]), ["campaign-y-summary.json", "campaign-y-summary.md"])
        self.assertEqual(outs[0], outs[1])
        self.assertIn("**Verdict: PASS**", outs[0]["campaign-y-summary.md"])


if __name__ == "__main__":
    unittest.main(verbosity=1)
