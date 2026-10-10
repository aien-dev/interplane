"""Against the real lithosai-motus 0.4.3 when it is installed (pip install '.[motus]').

Skipped (not failed) when Motus is absent; the CI-style run without Motus covers everything else.
A scripted chat client replaces the model, so no credentials or network are involved.
"""

import json

import pytest

# Import a real Motus submodule, not the bare name: a directory called "motus" (this
# adapter's own folder when pytest runs from adapters/) would satisfy the bare import.
pytest.importorskip("motus.agent", reason="lithosai-motus is not installed")

from motus.agent import ReActAgent  # noqa: E402
from motus.models.base import BaseChatClient, ChatCompletion, FunctionCall, ToolCall  # noqa: E402
from motus.runtime import resolve  # noqa: E402
from motus.tools import Tool, normalize_tools  # noqa: E402

from interplane_adapter_motus import host, motus_tool  # noqa: E402
from interplane_adapter_motus.verify import verify_bundle  # noqa: E402


class ScriptedClient(BaseChatClient):
    def __init__(self, script):
        self.script = list(script)
        self.seen_tools = []

    async def create(self, model, messages, tools=None, reasoning=None, **kwargs):
        self.seen_tools.append([t.name for t in tools or []])
        step = self.script.pop(0)
        if isinstance(step, str):
            return ChatCompletion(id="c", model=model, content=step)
        calls = [ToolCall(id=f"call_{i}", function=FunctionCall(name=n, arguments=a)) for i, (n, a) in enumerate(step)]
        return ChatCompletion(id="c", model=model, tool_calls=calls, finish_reason="tool_calls")

    async def parse(self, *a, **kw):
        raise NotImplementedError


def test_pin_matches_installed_version():
    assert motus_tool.pin_matches(), motus_tool.installed_version()


def test_tool_class_is_a_motus_tool_and_normalizes(make_local):
    authority, gate = make_local()
    tools = host.build_tools(gate, motus_tool.interplane_tool_class())
    assert all(isinstance(t, Tool) for t in tools.values())
    assert list(normalize_tools(list(tools.values()))) == list(tools)


def test_react_agent_dispatch_goes_through_the_gate(make_local, workspace):
    authority, gate = make_local()
    tools = host.build_tools(gate, motus_tool.interplane_tool_class())
    client = ScriptedClient([
        [("read_file", json.dumps({"path": "notes.txt"})),
         ("write_note", json.dumps({"path": "n.txt", "text": "x"})),
         ("read_file", "{bad json")],
        "done",
    ])
    agent = ReActAgent(client=client, model_name="scripted", tools=list(tools.values()))
    assert resolve(agent("read my notes")) == "done"
    assert client.seen_tools[0] == ["read_file", "list_dir", "write_note"]
    recs = gate.evidence()["records"]
    assert sorted(r["outcome"] for r in recs) == ["approval_pending", "finished", "rejected"]
    assert not (workspace / "n.txt").exists() and authority.execute_calls == 1
    assert verify_bundle(gate.evidence()) == []


def test_react_agent_through_the_gate_backed_by_real_aien(tmp_path):
    """Real Motus ReActAgent, scripted client, gate backed by AienBridgeAuthority (real AIEN)."""
    import os

    from interplane_adapter_motus.aien_bridge import ENV_BRIDGE, AienBridgeAuthority
    from interplane_adapter_motus.gate import Gate

    bridge = os.environ.get(ENV_BRIDGE)
    if not bridge or not os.access(bridge, os.X_OK):
        pytest.skip(f"aien-authority-bridge not built or {ENV_BRIDGE} unset")
    ws = tmp_path / "ws"
    ws.mkdir()
    (ws / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
    authority = AienBridgeAuthority(str(ws), bridge_path=bridge, timeout=20)
    assert authority.available, authority.fault
    try:
        gate = Gate(authority, authority.mapping_table())
        gate.register_user_turn("read my notes")
        tools = host.build_tools(gate, motus_tool.interplane_tool_class())
        client = ScriptedClient([
            [("read_file", json.dumps({"path": "notes.txt"})),
             ("write_file", json.dumps({"path": "n.txt", "content": "x"}))],
            "done",
        ])
        agent = ReActAgent(client=client, model_name="scripted", tools=list(tools.values()))
        assert resolve(agent("read my notes")) == "done"
        assert "write_file" in client.seen_tools[0]
        bundle = gate.evidence()
        assert sorted(r["outcome"] for r in bundle["records"]) == ["approval_pending", "finished"]
        assert not (ws / "n.txt").exists()
        assert bundle["header"]["runtime_id"] == authority.runtime_id == "aien"
        assert verify_bundle(bundle) == []
    finally:
        authority.close()
