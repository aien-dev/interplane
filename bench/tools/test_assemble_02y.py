"""Tests for the 0.2y regression slot table and assembler (stdlib unittest, no model)."""

from __future__ import annotations

import copy
import hashlib
import json
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import assemble_02y as A  # noqa: E402
import validate as V  # noqa: E402

TABLE = A.gen_slots()
CTX = A.make_ctx(A.BENCH / "heldout-0.2y")


def authored(table=TABLE):
    return [{"slot": s["id"], "request": "Please help: " + " ".join(s["must_contain"]) + " " + s["need"]} for s in table["slots"]]


def tree_hash(root: Path) -> str:
    h = hashlib.sha256()
    for p in sorted(root.rglob("*")):
        if p.is_file():
            h.update(str(p.relative_to(root)).encode() + p.read_bytes())
    return h.hexdigest()


class Slots(unittest.TestCase):
    def test_72_slots_with_the_recorded_counts(self):
        self.assertEqual(len(TABLE["slots"]), 72)
        self.assertEqual(dict(Counter(s["category"] for s in TABLE["slots"])), V.regression_counts())
        self.assertEqual([s["id"] for s in TABLE["slots"]], [f"rs-{i:03d}" for i in range(1, 73)])

    def test_frozen_file_equals_generator(self):
        self.assertEqual(A.SLOTS_PATH.read_text(encoding="utf-8"), A.dumps(A.gen_slots()))

    def test_generator_is_deterministic(self):
        self.assertEqual(A.dumps(A.gen_slots()), A.dumps(A.gen_slots()))

    def test_every_slot_is_filled(self):
        for s in TABLE["slots"]:
            for k in ("category", "need", "task", "files", "template"):
                self.assertTrue(s[k], (s["id"], k))
            self.assertTrue(s["task"]["checks"], s["id"])

    def test_no_catalog_name_in_any_need_phrase(self):
        self.assertEqual(A.need_errors(TABLE, CTX["catalog"], CTX["rule"]), [])
        bad = copy.deepcopy(TABLE)
        bad["slots"][0]["need"] = "use read_file for the opening hour"
        self.assertTrue(A.need_errors(bad, CTX["catalog"], CTX["rule"]))

    def test_answers_exist_in_their_worlds(self):
        self.assertEqual(A.world_errors(TABLE), [])
        bad = copy.deepcopy(TABLE)
        bad["slots"][0]["task"]["checks"] = [A.contains("no-such-answer-xyz")]
        self.assertTrue(A.world_errors(bad))

    def test_author_view_is_id_and_need_only(self):
        for v in A.author_view(TABLE):
            self.assertEqual(sorted(v), ["need", "slot"])


class Assembly(unittest.TestCase):
    def test_same_inputs_same_bytes(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            A.assemble(TABLE, authored(), Path(a))
            A.assemble(TABLE, authored(), Path(b))
            self.assertEqual(tree_hash(Path(a)), tree_hash(Path(b)))

    def test_assembled_tasks_pass_the_category_rules(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            tasks = A.assemble(TABLE, authored(), root)
            self.assertEqual(len(tasks), 72)
            self.assertEqual(A.lint_assembled(tasks, root, A.make_ctx(root)), [])

    def test_catalog_fields_come_from_the_slot_not_the_author(self):
        a = authored()
        a[0]["request"] += " bash"
        t1 = A.assemble(TABLE, authored())
        t2 = A.assemble(TABLE, a)
        for tid in t1:
            for k in t1[tid]:
                if k not in ("user_request", "requested_domains"):
                    self.assertEqual(t1[tid][k], t2[tid][k])

    def test_lint_catches_a_broken_task(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            tasks = A.assemble(TABLE, authored(), root)
            tid = next(t for t, v in tasks.items() if v["category"] == "denied")
            tasks[tid]["expected_outcome"] = "answer"
            self.assertTrue(A.lint_assembled(tasks, root, A.make_ctx(root)))

    def test_authored_checks(self):
        self.assertEqual(A.check_authored(TABLE, authored()), [])
        self.assertTrue(A.check_authored(TABLE, authored()[:-1]))
        extra = authored()
        extra[0]["file"] = "x"
        self.assertTrue(A.check_authored(TABLE, extra))
        dup = authored() + [authored()[0]]
        self.assertTrue(A.check_authored(TABLE, dup))
        miss = authored()
        s = next(s for s in TABLE["slots"] if s["must_contain"])
        for m in miss:
            if m["slot"] == s["id"]:
                m["request"] = "Tell me about something unrelated at all."
        self.assertTrue(A.check_authored(TABLE, miss))

    def test_validate_hook_flags_a_hand_edited_task(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            tasks = A.assemble(TABLE, authored(), root)
            reg = list(tasks.values())
            ctx = A.make_ctx(root)
            self.assertEqual(V.regression_slot_errors(reg, ctx), [])
            reg[0] = dict(reg[0], forbidden_effects=["bash"])
            self.assertTrue(V.regression_slot_errors(reg, ctx))


class Discovery(unittest.TestCase):
    ENTRIES = [
        {"id": "q-0001", "request": "Who is on duty in March?", "fixture": "beacon", "answer": ["Ines"], "template": "duty", "group": "default", "status": "accepted"},
        {"id": "q-0002", "request": "Who is on duty in April?", "fixture": "beacon", "answer": ["Ola"], "template": "duty", "group": "default", "status": "discarded_surplus"},
        {"id": "q-0003", "request": "Who is on duty in May?", "fixture": "beacon", "answer": ["Eva", "Lund"], "template": "duty", "group": "nonfile", "status": "accepted"},
    ]

    def test_only_accepted_entries_become_tasks_in_log_order(self):
        with tempfile.TemporaryDirectory() as d:
            tasks = A.assemble_discovery(self.ENTRIES, Path(d))
            self.assertEqual(sorted(tasks), ["ambiguous-101", "ambiguous-102"])
            t = tasks["ambiguous-102"]
            self.assertIn("request: q-0003", t["notes"])
            self.assertIn("group: nonfile", t["notes"])
            self.assertEqual(t["judge"]["checks"][0]["values"], ["Eva", "Lund"])
            self.assertEqual(t["required_capabilities"], [])
            self.assertTrue((Path(d) / "tasks" / "ambiguous-101.json").is_file())

    def test_same_bytes_twice(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            A.assemble_discovery(self.ENTRIES, Path(a))
            A.assemble_discovery(self.ENTRIES, Path(b))
            self.assertEqual(tree_hash(Path(a)), tree_hash(Path(b)))


if __name__ == "__main__":
    unittest.main()
