"""Optional access to a real Odysseus checkout (never modified, never vendored).

``load()`` imports Odysseus's own modules from the directory named by ``ODYSSEUS_SRC`` (or from
the current ``sys.path`` when ``src`` is already importable) and returns the handful of objects
the adapter calls. It returns ``None`` when Odysseus is not importable; callers must then fail
closed. Import order matters: ``src.agent_tools`` first, then ``src.tool_schemas`` (their circular
import only resolves in that order).
"""

from __future__ import annotations

import importlib
import os
import sys
import tempfile
from dataclasses import dataclass
from types import ModuleType
from typing import Any, Optional

ENV = "ODYSSEUS_SRC"


@dataclass(frozen=True)
class Odysseus:
    """The Odysseus modules and the few private names the adapter reuses."""

    agent_tools: ModuleType
    tool_schemas: ModuleType
    tool_security: ModuleType
    tool_capabilities: ModuleType
    tool_execution: ModuleType
    prompt_security: ModuleType
    root: str

    @property
    def tool_tags(self) -> Any:
        return self.agent_tools.TOOL_TAGS

    @property
    def tool_handlers(self) -> Any:
        return self.agent_tools.TOOL_HANDLERS


_cache: dict = {}


def load(src_dir: Optional[str] = None) -> Optional[Odysseus]:
    """Import Odysseus from ``src_dir`` / ``$ODYSSEUS_SRC`` / ``sys.path``; ``None`` if absent."""
    src_dir = src_dir or os.environ.get(ENV) or None
    key = os.path.realpath(src_dir) if src_dir else ""
    if key in _cache:
        return _cache[key]
    _cache[key] = result = _import(key)
    return result


def _import(root: str) -> Optional[Odysseus]:
    # Importing Odysseus's core.database creates data/app.db inside the checkout. Point its data
    # directory (src/constants.py:56, ODYSSEUS_DATA_DIR) at a scratch directory so the checkout is
    # never written; an operator-set value is respected.
    os.environ.setdefault("ODYSSEUS_DATA_DIR", tempfile.mkdtemp(prefix="interplane-odysseus-data-"))
    if root:
        if not os.path.isfile(os.path.join(root, "src", "tool_schemas.py")):
            return None
        if root not in sys.path:
            sys.path.insert(0, root)
    try:
        agent_tools = importlib.import_module("src.agent_tools")
        tool_schemas = importlib.import_module("src.tool_schemas")
        tool_security = importlib.import_module("src.tool_security")
        tool_capabilities = importlib.import_module("src.tool_capabilities")
        tool_execution = importlib.import_module("src.tool_execution")
        prompt_security = importlib.import_module("src.prompt_security")
    except Exception:  # noqa: BLE001 - any import failure means "not available"
        return None
    where = os.path.realpath(os.path.dirname(os.path.dirname(tool_schemas.__file__)))
    if root and where != root:
        return None  # a different `src` package shadowed the requested checkout
    return Odysseus(
        agent_tools, tool_schemas, tool_security, tool_capabilities, tool_execution,
        prompt_security, where,
    )
