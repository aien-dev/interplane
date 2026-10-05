"""The shared adapter runner (conformance/runners/adapter_runner.py) reproduces both recorded adapter
runs byte for byte: the independent to-do host and the maintainer's smart-home host, each with and
without its exposure check (the negative control)."""

import json
import shutil
import subprocess
import sys

import pytest

from conftest import ROOT

RUNNER = ROOT / "conformance" / "runners" / "adapter_runner.py"
TOY = ROOT / "bench" / "runs" / "adapter-repro-20261005T1215Z"
HOME = ROOT / "bench" / "runs" / "home-check-20261005T1320Z"

SHIM = """from authority import RUNTIME, POLICY, make_pipeline, {cls}


def make_authority(**kw):
    return {cls}(**kw)
"""


def toy_entry(tmp_path):
    """The to-do host's table entry, from the independent run's own diff of the shared table."""
    work = tmp_path / "table"
    (work / "conformance").mkdir(parents=True)
    shutil.copy(ROOT / "conformance" / "adapter-translation.json", work / "conformance")
    subprocess.run(["git", "apply", "--include=conformance/adapter-translation.json",
                    str(TOY / "conformance.diff")], cwd=work, check=True)
    table = json.loads((work / "conformance" / "adapter-translation.json").read_text())
    return {"toy": table["adapters"]["toy"]}


CASES = {
    # name: (evidence dir, authority class, (normal summary, negative-control summary))
    "toy": (TOY / "toy", "TodoAuthority", ((38, 0, 0, 27), (38, 18, 19, 27))),
    "home": (HOME, "HomeAuthority", ((43, 0, 0, 32), (43, 30, 32, 32))),
}


@pytest.mark.parametrize("name", sorted(CASES))
@pytest.mark.parametrize("negctl", [False, True])
def test_shared_runner_reproduces_recorded_run(tmp_path, name, negctl):
    evidence, cls, summaries = CASES[name]
    adapter = tmp_path / name
    adapter.mkdir()
    shutil.copy(evidence / "authority.py", adapter)
    (adapter / "shim.py").write_text(SHIM.format(cls=cls))
    entry = toy_entry(tmp_path) if name == "toy" else json.loads(
        (HOME / "table-entry.json").read_text())
    (adapter / "entry.json").write_text(json.dumps(entry))
    out = adapter / "verdicts.json"
    args = [sys.executable, str(RUNNER), "--module", str(adapter / "shim.py"),
            "--entry", str(adapter / "entry.json"), "--out", str(out)]
    if negctl:
        args.append("--no-exposure-check")
    run = subprocess.run(args, cwd=tmp_path, capture_output=True, text=True)
    assert run.returncode == (1 if negctl else 0), run.stderr
    s = json.loads(run.stdout)
    executed, failed, violations, intents = summaries[negctl]
    assert (s["executed"], s["failed"], s["violations"], s["injected_intents"]) == (
        executed, failed, violations, intents)
    assert s["subset_size"] == executed and s["content_derived"] == 0
    recorded = evidence / ("verdicts-negctl.json" if negctl else "verdicts.json")
    assert out.read_bytes() == recorded.read_bytes()


def test_shared_table_is_not_written(tmp_path):
    before = (ROOT / "conformance" / "adapter-translation.json").read_bytes()
    test_shared_runner_reproduces_recorded_run(tmp_path, "home", False)
    assert (ROOT / "conformance" / "adapter-translation.json").read_bytes() == before


def test_module_without_the_contract_is_refused(tmp_path):
    (tmp_path / "bad.py").write_text("RUNTIME = 'x'\n")
    (tmp_path / "entry.json").write_text((HOME / "table-entry.json").read_text())
    run = subprocess.run([sys.executable, str(RUNNER), "--module", str(tmp_path / "bad.py"),
                          "--entry", str(tmp_path / "entry.json")],
                         capture_output=True, text=True)
    assert run.returncode != 0 and "make_authority" in run.stderr
