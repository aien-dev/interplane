import json

import pytest

from interplane.core import jcs
from interplane.conformance import main, run_case, run_lifecycle_case
from conftest import CONF, load

CASES = sorted(CONF.glob("*.json"))
LIFECYCLE = sorted((CONF / "lifecycle").glob("*.json"))


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
    assert list(data) == sorted(data) and len(data) == len(CASES) + len(LIFECYCLE)
    life = {k: data.pop(k) for k in list(data) if k.startswith("lifecycle/")}
    assert len(life) == len(LIFECYCLE)
    assert all(v["pass"] and set(v) == {"pass", "steps"} for v in life.values())
    base = {"pass", "observed", "runtime", "turns"}
    assert all(v["pass"] and set(v) - {"selections"} == base for v in data.values())
    assert {k for k, v in data.items() if "selections" in v} == {
        "24-expansion-requested-excluded",
        "25-expansion-refused-by-bound",
    }
    assert text == jcs({**data, **life}) + "\n"
    assert all(o["result_digest"] for v in data.values() for o in v["observed"])
    assert "30/30" in capsys.readouterr().out or len(CASES) + len(LIFECYCLE) != 30


@pytest.mark.skipif(not LIFECYCLE, reason="no lifecycle fixtures")
@pytest.mark.parametrize("path", LIFECYCLE, ids=lambda p: p.stem)
def test_lifecycle_case_passes(path):
    verdict = run_lifecycle_case(load(path))
    assert verdict["problems"] == []


@pytest.mark.skipif(not LIFECYCLE, reason="no lifecycle fixtures")
def test_lifecycle_runner_fails_on_mismatch():
    case = load(LIFECYCLE[0])
    case["expected"][1]["refused"] = False
    assert run_lifecycle_case(case)["pass"] is False


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
def test_runner_fails_on_mismatch(tmp_path):
    case = load(CASES[0])
    case["expected"]["observed"][0]["status"] = "denied"
    d = tmp_path / "f"
    d.mkdir()
    (d / "x.json").write_text(json.dumps(case))
    assert main([str(d), "--out", str(tmp_path / "o.json")]) == 1
