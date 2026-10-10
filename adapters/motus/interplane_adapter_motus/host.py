"""Host side of the seam: tools Motus can call, and a tiny fake host for when Motus is absent.

Motus 0.4.3 dispatches ``self.tools[call.function.name](call.function.arguments)`` and awaits the
returned object (react_agent.py:241 and :217-221). A tool therefore needs: ``name``,
``description``, ``json_schema`` and ``__call__(args: str)`` returning an awaitable of ``str``
(tools/core/tool.py:20-30 and :72). ``FakeHostTool`` has exactly that shape and nothing else.
"""

from __future__ import annotations

import asyncio
import json
from typing import Any, Callable, Optional

from .gate import Gate


class ToolBridge:
    """Runs one gate call off the event loop and keeps cancellation honest."""

    def __init__(self, gate: Gate, tool_name: str, motus_ids: Optional[Callable[[], dict]] = None) -> None:
        self.gate = gate
        self.tool_name = tool_name
        self._motus_ids = motus_ids

    async def invoke(self, raw_arguments: str) -> str:
        call_id = self.gate.new_call_id()
        ids = self._motus_ids() if self._motus_ids else None
        work = asyncio.ensure_future(
            asyncio.to_thread(self.gate.call, self.tool_name, raw_arguments, call_id=call_id, motus_ids=ids)
        )
        try:
            outcome = await asyncio.shield(work)
        except asyncio.CancelledError:
            # The worker thread cannot be recalled. Say so in the evidence; never run it again.
            self.gate.note_cancellation(call_id)
            raise
        return outcome.text


class FakeHostTool:
    """The least a Motus-shaped host needs from a tool. Not Motus."""

    def __init__(self, bridge: ToolBridge, name: str, description: str, json_schema: dict) -> None:
        self.name = name
        self.description = description
        self.json_schema = json_schema
        self._bridge = bridge

    def __call__(self, args: str):
        return self._bridge.invoke(args if isinstance(args, str) else "")


def build_tools(gate: Gate, tool_cls: Any = FakeHostTool, motus_ids: Optional[Callable[[], dict]] = None) -> dict:
    """One host tool per capability the authority advertises. Nothing else is exposed."""
    tools = {}
    for entry in gate.exposed_tools():
        bridge = ToolBridge(gate, entry["name"], motus_ids)
        tools[entry["name"]] = tool_cls(bridge, entry["name"], entry["description"], entry["parameters"])
    return tools


def dispatch(tools: dict, name: str, arguments: dict | str) -> Any:
    """What the Motus ReAct loop does with a model tool call (react_agent.py:241)."""
    raw = arguments if isinstance(arguments, str) else json.dumps(arguments)
    return tools[name](raw)
