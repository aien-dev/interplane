"""Process-launch preflight: would Odysseus's launch boundary structurally accept this workspace?

Odysseus gates a workspace twice. Selection (``src.tool_execution.vet_workspace``, tool_execution.py
:1179 at a8c147b) accepts any real directory that is not sensitive or app state, including an
ANCESTOR of ``ODYSSEUS_DATA_DIR`` (deliberate, see ``_is_app_state_path``). The launch guard
(``src.agent_runtime.process_resources.guard_launch_workspace``, :379 at a8c147b) refuses a root
that contains server control state or aliases it, but does NOT refuse a root nested inside
``DATA_DIR`` (selection does). Neither gate alone is the whole answer (odysseus-dev/odysseus#6651),
so this module asks both, in the real upstream code, read-only.

A positive result is structural only. It is never an authorization and never a claim that anything
ran or will run: Odysseus still decides at launch (tool permission, admin, delegated credentials,
untrusted context, approvals). Nothing here runs a handler, writes a file or changes the
environment. No field ever carries a path or upstream exception text.
"""

from __future__ import annotations

import importlib
import os
from dataclasses import dataclass
from typing import Any, Optional

PROCESS_TOOLS = frozenset({"bash", "python", "host_shell", "manage_bg_jobs"})

_FIX = {
    "runtime_unavailable": ("runtime", "install Odysseus and set ODYSSEUS_SRC to its checkout; until then no process launch is possible"),
    "not_a_process_tool": ("request", "ask about one of: bash, python, host_shell, manage_bg_jobs; read-only tools need no launch preflight"),
    "process_tool_unavailable": ("runtime", "this Odysseus build does not register the requested process tool; use a revision that does"),
    "host_api_unsupported": ("runtime", "this Odysseus revision has no launch-boundary guard the adapter can ask (qualified: a8c147b); treat process launch as not ready"),
    "workspace_missing": ("selection", "name a workspace directory"),
    "workspace_invalid": ("selection", "the workspace must be an existing directory; check the path"),
    "workspace_inaccessible": ("selection", "grant the Odysseus user read and search access to the workspace, or choose another directory"),
    "workspace_is_server_state": ("selection", "choose a workspace that is not Odysseus's data directory; set ODYSSEUS_DATA_DIR outside every agent workspace"),
    "workspace_within_server_state": ("selection", "choose a workspace outside Odysseus's data directory; the adapter never moves existing state"),
    "workspace_rejected_by_host": ("selection", "Odysseus itself refuses this directory as a workspace (sensitive, app state or filesystem root); choose another"),
    "indeterminate": ("launch_boundary", "the launch boundary could not be inspected; fix the workspace permissions or choose a smaller, readable workspace, then retry"),
    "workspace_contains_server_state": ("launch_boundary", "choose a workspace that does not contain Odysseus's data directory, or set ODYSSEUS_DATA_DIR outside it; the adapter never moves existing state"),
    "workspace_aliases_server_state": ("launch_boundary", "remove the symlink or hardlink inside the workspace that points at Odysseus server state, or choose another workspace"),
    "structurally_eligible": ("preflight", "structurally eligible only; Odysseus still decides at launch (tool permission, admin, delegated credentials, untrusted context, approvals)"),
}
REASON_CODES = frozenset(_FIX)


@dataclass(frozen=True)
class LaunchPreflight:
    """Result of ``launch_preflight``. ``eligible`` is structural only: never an authorization,
    never a claim that a process was or will be started. Fields hold fixed text, no paths."""

    eligible: bool
    code: str
    stage: str
    remediation: str


def _result(code: str) -> LaunchPreflight:
    stage, fix = _FIX[code]
    return LaunchPreflight(code == "structurally_eligible", code, stage, fix)


def launch_preflight(ody: Any, workspace: Optional[str], tool: str = "bash") -> LaunchPreflight:
    """Return the first failing check, in order runtime, selection, launch boundary."""
    code = _runtime(ody, tool)
    if code is not None:
        return _result(code)
    api = _host_api()
    if api is None:
        return _result("host_api_unsupported")
    code = _selection(ody, workspace)
    if code is not None:
        return _result(code)
    return _result(_boundary(api, os.path.realpath(workspace)) or "structurally_eligible")


def _runtime(ody: Any, tool: str) -> Optional[str]:
    if ody is None:
        return "runtime_unavailable"
    if tool not in PROCESS_TOOLS:
        return "not_a_process_tool"
    try:
        if tool not in ody.tool_tags or tool not in ody.tool_handlers:
            return "process_tool_unavailable"
    except Exception:  # noqa: BLE001 - an unreadable registry is not a ready runtime
        return "process_tool_unavailable"
    return None


def _host_api() -> Optional[tuple]:
    """(guard_launch_workspace, FilesystemRoot, ResourceIdentityError) or None if absent."""
    try:
        pr = importlib.import_module("src.agent_runtime.process_resources")
        res = importlib.import_module("src.agent_runtime.resources")
        guard = pr.guard_launch_workspace
        root, err = res.FilesystemRoot, res.ResourceIdentityError
        root.seal
    except Exception:  # noqa: BLE001 - any failure means the API is not there
        return None
    return guard, root, err


def _inside(path: str, base: str) -> bool:
    return os.path.commonpath([path, base]) == base


def _selection(ody: Any, workspace: Optional[str]) -> Optional[str]:
    if not workspace or not isinstance(workspace, str):
        return "workspace_missing"
    real = os.path.realpath(workspace)
    if not os.path.isdir(real):
        return "workspace_invalid"
    if not os.access(real, os.R_OK | os.X_OK):
        return "workspace_inaccessible"
    try:
        data = os.path.realpath(importlib.import_module("src.constants").DATA_DIR)
    except Exception:  # noqa: BLE001
        return "host_api_unsupported"
    if real == data:
        return "workspace_is_server_state"
    if _inside(real, data):
        return "workspace_within_server_state"
    try:
        vetted = ody.tool_execution.vet_workspace(workspace)
    except Exception:  # noqa: BLE001
        vetted = None
    if vetted is None or os.path.realpath(vetted) != real:
        return "workspace_rejected_by_host"
    return None


def _boundary(api: tuple, real: str) -> Optional[str]:
    guard, filesystem_root, error = api
    try:
        root = filesystem_root.seal(real)
    except Exception:  # noqa: BLE001
        return "indeterminate"
    try:
        guard(root)
    except error as exc:
        text = str(exc)
        if "contains" in text:
            return "workspace_contains_server_state"
        if "aliases" in text:
            return "workspace_aliases_server_state"
        return "indeterminate"
    except Exception:  # noqa: BLE001 - OSError and anything else
        return "indeterminate"
    return None
