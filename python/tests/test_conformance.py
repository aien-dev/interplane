import json

import pytest

from interplane.core import jcs
from interplane.conformance import main, run_case, run_lifecycle_case
from conftest import CONF, load

CASES = sorted(CONF.glob("*.json"))
LIFECYCLE = sorted((CONF / "lifecycle").glob("*.json"))
INJECTION = sorted((CONF / "injection").glob("*.json"))
APPROVAL = sorted((CONF / "approval").glob("*.json"))


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
@pytest.mark.parametrize("path", CASES, ids=lambda p: p.stem)
def test_case_passes(path):
    verdict = run_case(load(path))
    assert verdict["problems"] == []


@pytest.mark.skipif(not APPROVAL, reason="no approval fixtures")
@pytest.mark.parametrize("path", APPROVAL, ids=lambda p: p.stem)
def test_approval_case_passes(path):
    verdict = run_case(load(path))
    assert verdict["problems"] == []


@pytest.mark.skipif(not APPROVAL, reason="no approval fixtures")
def test_approval_runner_fails_on_mismatch():
    case = load(APPROVAL[0])
    case["expected"]["continuations"][0]["outcome"] = "refused"
    assert run_case(case)["pass"] is False
    case = load(APPROVAL[0])
    case["expected"]["runtime"]["execute_calls"] = 0
    assert run_case(case)["pass"] is False


@pytest.mark.skipif(not CASES, reason="no conformance fixtures")
def test_runner_writes_sorted_verdicts(tmp_path, capsys):
    out = tmp_path / "v.json"
    assert main([str(CONF), "--out", str(out)]) == 0
    text = out.read_text(encoding="utf-8")
    data = json.loads(text)
    assert list(data) == sorted(data) and len(data) == len(CASES) + len(LIFECYCLE) + len(INJECTION) + len(APPROVAL)
    life = {k: data.pop(k) for k in list(data) if k.startswith("lifecycle/")}
    assert len(life) == len(LIFECYCLE)
    assert all(v["pass"] and set(v) == {"pass", "steps"} for v in life.values())
    base = {"pass", "observed", "runtime", "turns"}
    optional = {"selections", "inputs", "exposure", "injection", "continuations"}
    assert all(v["pass"] and set(v) - optional == base for v in data.values())
    ledger_cases = {
        "28-input-ledger-continues",
        "29-input-derived-from",
        "30-input-forged-exposure",
        "31-source-class-workspace",
        "32-source-class-web",
        "33-source-class-memory",
        "34-source-class-document",
        "35-source-class-skill",
        "36-source-class-tool-output",
        "37-source-class-external-provider",
        "38-source-class-runtime-generated",
        "39-source-class-user-request",
        "40-source-class-model-generated",
    }
    assert {k for k, v in data.items() if "inputs" in v and "exposure" in v} == ledger_cases
    assert {k for k, v in data.items() if "injection" in v} == {
        "injection-01-workspace-write",
        "injection-02-tool-output-markup",
        "injection-03-forged-approval-in-arguments",
    }
    assert {k for k, v in data.items() if "continuations" in v} == {
        load(p)["case"]
        for p in APPROVAL
        if load(p)["expected"]["continuations"]
    }
    assert {k for k, v in data.items() if "selections" in v} == {
        "24-expansion-requested-excluded",
        "25-expansion-refused-by-bound",
    }
    assert text == jcs({**data, **life}) + "\n"
    assert all(o["result_digest"] for v in data.values() for o in v["observed"])
    total = len(CASES) + len(LIFECYCLE) + len(INJECTION) + len(APPROVAL)
    assert f"{total}/{total}" in capsys.readouterr().out


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
