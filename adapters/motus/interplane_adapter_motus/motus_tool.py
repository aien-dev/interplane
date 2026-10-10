"""Optional Motus binding. Importing this module never imports Motus.

``interplane_tool_class()`` imports the public ``Tool`` base class (``motus.tools``, re-exported
from ``motus/tools/core/tool.py:10``) only when called. If Motus is not installed it raises
``MotusNotInstalled`` and the host stays on ``FakeHostTool``: fail closed, nothing authorized.
"""

from __future__ import annotations

import importlib.metadata
import json
from typing import Optional

from . import MOTUS_PIN


class MotusNotInstalled(RuntimeError):
    pass


def installed_version() -> Optional[str]:
    try:
        return importlib.metadata.version("lithosai-motus")
    except importlib.metadata.PackageNotFoundError:
        return None


def pin_matches() -> bool:
    return installed_version() == MOTUS_PIN.split("==")[1]


def interplane_tool_class():
    """A ``motus.tools.Tool`` subclass whose calls go through the gate instead of running code."""
    try:
        from motus.runtime.agent_task import agent_task  # tools/core/tool.py:6
        from motus.runtime.types import TOOL_CALL  # tools/core/tool.py:7
        from motus.tools.core.tool import Tool  # tools/core/tool.py:10
    except Exception as err:  # noqa: BLE001 - any import failure means "not usable here"
        raise MotusNotInstalled("lithosai-motus is not importable; install the 'motus' extra") from err

    class InterplaneTool(Tool):
        """Holds no implementation. The model's arguments go to the gate as the raw string."""

        def __init__(self, bridge, name, description, json_schema) -> None:
            super().__init__(name=name, description=description, json_schema=json_schema)
            self._bridge = bridge

        def __call__(self, args: str):
            return self._interplane_call(args if isinstance(args, str) else "")

        @agent_task(task_type=TOOL_CALL)
        async def _interplane_call(self, args: str) -> str:
            return await self._bridge.invoke(args)

        async def _invoke(self, **kwargs):  # abstract in Tool (tool.py:114); not used by __call__
            return await self._bridge.invoke(json.dumps(kwargs))

    return InterplaneTool
