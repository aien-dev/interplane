"""Against the real lithosai-motus 0.4.3 when it is installed (pip install '.[motus]').

Skipped (not failed) when Motus is absent; the CI-style run without Motus covers everything else.
A scripted chat client replaces the model, so no credentials or network are involved.
"""

import json

import pytest

motus = pytest.importorskip("motus")

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
