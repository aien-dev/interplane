"""Offline tests of bench/tools/authoring_02y.py (fact hand-over, E/Q split, pool size). Stdlib only."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import authoring_02y as A  # noqa: E402


def area(i):
    """Alternating tags (odd serial = an everyday area, even serial = other) keep the attempt-1 counts."""
    return A.AREAS[(i // 2) % len(A.AREAS)] if i % 2 == 0 else A.OTHER


def ws(slug, n):
    return {"slug": slug, "files": {"a/notes.txt": "x"},
            "facts": [{"fact": f"Fact {i} of {slug}.", "file": "a/notes.txt", "area": area(i)} for i in range(n)]}


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

    def test_the_area_tag_decides_the_split_not_the_position(self):
        w = ws("a", 40)
        for f in w["facts"][:20]:
            f["area"] = A.OTHER
        for i, f in enumerate(w["facts"][20:]):
            f["area"] = A.AREAS[i % 4]
        rec = A.split_record([w])
        self.assertEqual(rec["question"], [f"a-{n:03d}" for n in range(1, 21)])
        self.assertEqual(rec["everyday"], [f"a-{n:03d}" for n in range(21, 41)])
        self.assertEqual(A.fact_errors([w]), [])

    def test_missing_or_unknown_area_is_refused(self):
        w = ws("a", 40)
        del w["facts"][0]["area"]
        w["facts"][1]["area"] = "email"  # a word, not one of the listed areas
        errs = A.fact_errors([w])
        self.assertTrue(any(e.startswith("a-001: area None") for e in errs), errs)
        self.assertTrue(any(e.startswith("a-002: area 'email'") for e in errs), errs)

    def test_workspace_balance_is_enforced(self):
        self.assertEqual(A.fact_errors([ws("a", 40)]), [])
        lopsided = ws("b", 40)
        lopsided["facts"][0]["area"] = A.OTHER  # fact 0 was everyday: now 19 everyday, 21 other
        self.assertTrue(any(e.startswith("b: 19 everyday facts") for e in A.fact_errors([lopsided])))
        narrow = ws("c", 40)
        for f in narrow["facts"]:
            if f["area"] != A.OTHER:
                f["area"] = A.AREAS[0] if f is narrow["facts"][0] else A.AREAS[1]
        self.assertTrue(any("in 2 areas" in e for e in A.fact_errors([narrow])))

    def test_facts_with_paths_or_file_names_are_flagged(self):
        w = ws("a", 40)
        w["facts"][0]["fact"] = "It is in notes.txt."
        w["facts"][1]["fact"] = "See docs/rota."
        w["facts"][2]["fact"] = "The depot opens at nine."
        errs = A.fact_errors([w], r"\.(md|txt)(?![a-z0-9_])")
        self.assertEqual(len(errs), 2, errs)
        self.assertTrue(errs[0].startswith("a-001") and errs[1].startswith("a-002"))


class BriefsAreClean(unittest.TestCase):
    def test_no_author_facing_brief_says_tool(self):
        import re
        text = (Path(__file__).resolve().parent.parent / "heldout-0.2y" / "AUTHORING.md").read_text(encoding="utf-8")
        sec = text[text.index("## 4. Verbatim briefs"):text.index("## 5. Procedure")]
        blocks = re.findall(r"```text\n(.*?)```", sec, flags=re.S)
        self.assertGreaterEqual(len(blocks), 5)  # 4.1, 4.2 E and Q, 4.3 paragraph, 4.4
        for b in blocks:
            self.assertNotIn("tool", b.lower(), b[:80])


if __name__ == "__main__":
    unittest.main(verbosity=1)
