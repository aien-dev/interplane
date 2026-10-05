"""Offline tests of bench/tools/authoring_02y.py (fact hand-over, E/Q split, pool size). Stdlib only."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import authoring_02y as A  # noqa: E402


def ws(slug, n):
    return {"slug": slug, "files": {"a/notes.txt": "x"}, "facts": [{"fact": f"Fact {i} of {slug}.", "file": "a/notes.txt"} for i in range(n)]}


class Authoring(unittest.TestCase):
    def test_need_and_workspace_count(self):
        self.assertEqual(A.per_brief_need(), 108)  # 36 + 12 + 24 + 36
        self.assertEqual(A.workspaces_needed(), 6)  # ceil(216 / 40)

    def test_author_never_sees_files(self):
        seen = A.facts_for_author([ws("a", 10)], "everyday")
        self.assertTrue(seen)
        for f in seen:
            self.assertEqual(sorted(f), ["fact", "id"])
            self.assertNotIn("notes", repr(f))

    def test_subsets_are_disjoint_deterministic_and_cover_everything(self):
        w = [ws("a", 40), ws("b", 41)]
        rec = A.split_record(w)
        self.assertFalse(set(rec["everyday"]) & set(rec["question"]))
        self.assertEqual(len(rec["everyday"]) + len(rec["question"]), 81)
        self.assertEqual(rec, A.split_record(w))
        self.assertEqual(rec["everyday"][0], "a-001")
        self.assertEqual(rec["question"][0], "a-002")
        self.assertEqual({f["id"] for f in A.facts_for_author(w, "question")}, set(rec["question"]))

    def test_used_facts_are_removed_for_a_top_up(self):
        w = [ws("a", 40)]
        self.assertEqual(len(A.facts_for_author(w, "everyday", used={"a-001", "a-003"})), 18)

    def test_pool_check_passes_and_computes_the_enlargement(self):
        full = [ws(f"w{i}", 40) for i in range(6)]
        r = A.pool_check(full)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["available"], {"everyday": 120, "question": 120})
        small = [ws(f"w{i}", 40) for i in range(5)]  # 100 per brief, 8 short
        r = A.pool_check(small)
        self.assertFalse(r["ok"])
        self.assertEqual(r["shortfall_per_brief"], {"everyday": 8, "question": 8})
        self.assertEqual(r["extra_workspaces"], 1)  # ceil(8 / 20)
        r = A.pool_check([ws("a", 40)])
        self.assertEqual(r["extra_workspaces"], 5)  # ceil(88 / 20)
        odd = A.pool_check([ws(f"w{i}", 39) for i in range(5)])  # odd counts favour everyday: 100 vs 95
        self.assertFalse(odd["ok"])
        self.assertEqual(odd["extra_workspaces"], 1)

    def test_facts_with_paths_or_file_names_are_flagged(self):
        w = ws("a", 3)
        w["facts"][0]["fact"] = "It is in notes.txt."
        w["facts"][1]["fact"] = "See docs/rota."
        w["facts"][2]["fact"] = "The depot opens at nine."
        errs = A.fact_errors([w], r"\.(md|txt)(?![a-z0-9_])")
        self.assertEqual(len(errs), 2, errs)
        self.assertTrue(errs[0].startswith("a-001") and errs[1].startswith("a-002"))


if __name__ == "__main__":
    unittest.main(verbosity=1)
