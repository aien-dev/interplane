import json

import pytest

from interplane.core import jcs
from interplane.conformance import main, run_case
from conftest import CONF, load

CASES = sorted(CONF.glob("*.json"))


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
@pytest.mark.parametrize("path", CASES, ids=lambda p: p.stem)
def test_case_passes(path):
    verdict = run_case(load(path))
    assert verdict["problems"] == []


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
def test_runner_writes_sorted_verdicts(tmp_path, capsys):
    out = tmp_path / "v.json"
    assert main([str(CONF), "--out", str(out)]) == 0
    text = out.read_text(encoding="utf-8")
    data = json.loads(text)
    assert list(data) == sorted(data) and len(data) == len(CASES)
    assert all(
        v["pass"] and set(v) == {"pass", "observed", "runtime", "turns"} for v in data.values()
    )
    assert text == jcs(data) + "\n"
    assert all(o["result_digest"] for v in data.values() for o in v["observed"])
    assert "18/18" in capsys.readouterr().out or len(CASES) != 18


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
def test_runner_fails_on_mismatch(tmp_path):
    case = load(CASES[0])
    case["expected"]["observed"][0]["status"] = "denied"
    d = tmp_path / "f"
    d.mkdir()
    (d / "x.json").write_text(json.dumps(case))
    assert main([str(d), "--out", str(tmp_path / "o.json")]) == 1
