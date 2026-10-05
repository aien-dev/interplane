"""Offline tests: run_bench.py accepts --corpus 0.2y and 0.2y-cal and refuses missing, empty or partial corpora.

No model, no Odysseus, no GPU. Stdlib unittest:  PYTHONPATH=python python3 bench/tools/test_runner_corpus_y.py
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run_bench as R  # noqa: E402


def make_dir(root: Path, n: int, split: str = "qual") -> Path:
    d = root / "tasks"
    d.mkdir(parents=True, exist_ok=True)
    for i in range(n):
        (d / f"t-{i:03d}.json").write_text(json.dumps({"id": f"t-{i:03d}", "split": split}), encoding="utf-8")
    return d


def check(corpus: str, tasks_dir: Path, *extra) -> tuple:
    """(exit message or 0, stdout) of `run_bench --corpus X --check-corpus --tasks-dir D`."""
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            rc = R.main(["--corpus", corpus, "--check-corpus", "--backends", "sim-1", "--out", "/nonexistent/out",
                         "--tasks-dir", str(tasks_dir), *extra])
    except SystemExit as e:
        rc = e.code
    return rc, buf.getvalue()


class CorpusOption(unittest.TestCase):
    def test_both_corpora_are_accepted_choices(self):
        self.assertEqual(R.Y_CORPORA, ("0.2y", "0.2y-cal"))
        with self.assertRaises(SystemExit) as cm:
            R.main(["--corpus", "0.2z", "--out", "x"])
        self.assertEqual(cm.exception.code, 2)  # argparse rejects an unknown corpus

    def test_full_corpus_loads(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(check("0.2y", make_dir(Path(tmp), 120))[0], 0)
        with tempfile.TemporaryDirectory() as tmp:
            rc, out = check("0.2y-cal", make_dir(Path(tmp), 30))
            self.assertEqual(rc, 0)
            self.assertIn("30 tasks load", out)

    def test_missing_folder_is_refused(self):
        rc, _ = check("0.2y", Path("/nonexistent/tasks"))
        self.assertIn("does not exist", str(rc))

    def test_empty_folder_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp) / "tasks"
            d.mkdir()
            self.assertIn("holds no tasks", str(check("0.2y", d)[0]))
            self.assertIn("holds no tasks", str(check("0.2y-cal", d, "--allow-nonfrozen")[0]))

    def test_partial_corpus_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = make_dir(Path(tmp), 119)
            self.assertIn("partial corpus", str(check("0.2y", d)[0]))
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIn("expected exactly 30", str(check("0.2y-cal", make_dir(Path(tmp), 29))[0]))
        with tempfile.TemporaryDirectory() as tmp:  # the target count is wrong for the calibration set
            self.assertIn("expected exactly 30", str(check("0.2y-cal", make_dir(Path(tmp), 120))[0]))

    def test_partial_is_allowed_only_for_synthetic_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(check("0.2y", make_dir(Path(tmp), 5), "--allow-nonfrozen")[0], 0)

    def test_no_qual_tasks_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIn("no task has split", str(check("0.2y", make_dir(Path(tmp), 120, "dev"))[0]))

    def test_repository_corpus_without_tasks_is_refused(self):
        # the default tasks folder of 0.2y does not exist until the authoring PR merges
        import validate
        if (validate.CORPORA["0.2y"]["tasks"]).is_dir():
            self.skipTest("0.2y tasks exist in this checkout")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), self.assertRaises(SystemExit) as cm:
            R.main(["--corpus", "0.2y", "--check-corpus", "--backends", "sim-1", "--out", "/nonexistent/out"])
        self.assertIn("does not exist", str(cm.exception.code))

    def test_digest_file_is_required_for_a_real_run(self):
        # a corpus whose frozen digest file is absent fails check_corpus in strict mode
        import validate
        if validate.CORPORA["0.2y-cal"]["digest"].exists():
            self.skipTest("calibration digest exists in this checkout")
        with self.assertRaises(SystemExit) as cm:
            R.check_corpus(True, "0.2y-cal")
        self.assertIn("differs from CORPUS-DIGEST.txt", str(cm.exception))

    def test_check_corpus_flag_is_only_for_y_corpora(self):
        with self.assertRaises(SystemExit) as cm:
            R.main(["--corpus", "0.2", "--check-corpus", "--out", "x"])
        self.assertIn("0.2y corpora", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
