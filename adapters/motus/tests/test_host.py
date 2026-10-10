import asyncio
import json
import pathlib
import threading
import time

import pytest

from interplane_adapter_motus import host, motus_tool
from interplane_adapter_motus.gate import Gate
from interplane_adapter_motus.ids import TRACE_RE
from interplane_adapter_motus.verify import verify_bundle

J = json.dumps
SRC = pathlib.Path(__file__).resolve().parents[1] / "interplane_adapter_motus"


def test_tools_exposed_are_exactly_the_authority_catalog(make_local):
    authority, gate = make_local()
    tools = host.build_tools(gate)
    assert set(tools) == {c.name for c in authority.catalog().capabilities}
    assert tools["read_file"].json_schema == authority.catalog().get("read_file").parameters


def test_fake_host_roundtrip(make_local):
    authority, gate = make_local()
    tools = host.build_tools(gate)
    text = asyncio.run(host.dispatch(tools, "read_file", {"path": "notes.txt"}))
    assert json.loads(text)["content"].startswith("hello")
    text = asyncio.run(host.dispatch(tools, "write_note", {"path": "z.txt", "text": "x"}))
    assert json.loads(text)["status"] == "requires_approval" and authority.execute_calls == 1


def test_trace_id_is_minted_once_per_run_and_validated(make_local):
    _, gate = make_local()
    assert TRACE_RE.fullmatch(gate.trace_id)
    with pytest.raises(ValueError):
        Gate(None, None, trace_id="NOT-HEX")


def test_cancelling_the_motus_task_mid_call_is_recorded_and_not_retried(make_local):
    started, release = threading.Event(), threading.Event()

    def fault(req):
        started.set()
        release.wait(5)

    authority, gate = make_local(fault=fault)
    tools = host.build_tools(gate)

    async def scenario():
        task = asyncio.ensure_future(tools["read_file"](J({"path": "notes.txt"})))
        for _ in range(100):
            if started.is_set():
                break
            await asyncio.sleep(0.02)
        assert started.is_set()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        release.set()

    asyncio.run(scenario())
    for _ in range(100):
        if gate.evidence()["records"]:
            break
        time.sleep(0.02)
    rec = gate.evidence()["records"][-1]
    assert rec["outcome"] == "cancelled" and rec["effect_certainty"] == "uncertain"
    assert authority.execute_calls == 1
    assert verify_bundle(gate.evidence()) == []


def test_motus_not_installed_fails_closed(monkeypatch):
    import sys
    for name in [n for n in sys.modules if n == "motus" or n.startswith("motus.")] + ["motus"]:
        monkeypatch.setitem(sys.modules, name, None)  # None makes the import raise ImportError
    with pytest.raises(motus_tool.MotusNotInstalled):
        motus_tool.interplane_tool_class()


def test_importing_the_adapter_does_not_import_motus():
    import subprocess, sys
    code = "import sys, interplane_adapter_motus.demo, interplane_adapter_motus.host; sys.exit('motus' in sys.modules)"
    assert subprocess.run([sys.executable, "-c", code]).returncode == 0


def test_adapter_code_never_builds_a_decision_or_a_result():
    for name in ("gate.py", "host.py", "motus_tool.py", "verify.py", "evidence.py", "mapping.py", "ids.py"):
        text = (SRC / name).read_text()
        for needle in ("Decision(", "ToolResult(", "make_result(", "AuthorizedEffect"):
            assert needle not in text, (name, needle)


def test_no_dashes_in_adapter_text():
    root = SRC.parent
    for path in list(root.rglob("*.py")) + [root / "README.md"]:
        text = path.read_text()
        assert chr(0x2014) not in text and chr(0x2013) not in text, path


def test_demo_runs_and_its_bundle_verifies(tmp_path, capsys):
    from interplane_adapter_motus import demo
    out = tmp_path / "bundle.json"
    assert demo.main([str(out)]) == 0
    assert verify_bundle(json.loads(out.read_text())) == []
    printed = capsys.readouterr().out
    assert "LOCAL STAND-IN" in printed and "refused" in printed
