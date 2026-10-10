"""TEST DOUBLE, never ship. LocalReadOnlyAuthority: a LOCAL STAND-IN, not AIEN.

This lives under tests/ on purpose. It mints \"authorized\" decisions in Python, which the adapter must
never do; it exists only so the gate can be exercised offline. It is not importable from the package
and the demo does not use it.

The real authority is AIEN's, reached through ``aien-authority-bridge`` (``adapters/aien``). This class is a small,
separate ``RuntimeAuthority`` implementation so the Motus path can be exercised end to end:

* three capabilities: ``read_file`` and ``list_dir`` (read only) and ``write_note`` (creates a
  new file; always held for approval, never executed without a continuation);
* every path is confined to one workspace root (no ``..``, no absolute paths outside, no symlinks);
* anything unknown, malformed or unconfirmable is refused (fail closed).

It is the authority, so it is the one place decisions are made. The adapter around it never makes one.
"""

from __future__ import annotations

import copy
import os
from pathlib import Path
from typing import Callable, Optional

from interplane.core import (
    CapabilityDescriptor,
    CapabilityRequest,
    Catalog,
    Decision,
    ErrorCode,
    ToolRef,
    ToolResult,
)
from interplane.crossveil import PendingApproval, make_result

RUNTIME_ID = "local-readonly"
POLICY_ENGINE = "interplane-adapter-motus.local-readonly"
LABEL = "LOCAL STAND-IN (not AIEN): live AIEN-backed path not executed"
NOT_EXECUTED = "not executed: decision was not authorized for this request"
MAX_READ = 65536
MAX_NOTE = 4096

_PATH = {"type": "string"}
_SPECS = {
    "read_file": ("Read a text file inside the workspace.", {"path": _PATH}, ["path"]),
    "list_dir": ("List a directory inside the workspace.", {"path": _PATH}, ["path"]),
    "write_note": (
        "Create a new text file inside the workspace (held for human approval).",
        {"path": _PATH, "text": {"type": "string"}},
        ["path", "text"],
    ),
}


class UncertainEffect(Exception):
    """Raised by a fault hook to model a lost response: the effect may or may not have happened."""


def _schema(props: dict, required: list) -> dict:
    return {"type": "object", "properties": copy.deepcopy(props), "required": list(required),
            "additionalProperties": False}


def build_catalog() -> Catalog:
    caps = [
        CapabilityDescriptor(
            name=name,
            description=desc,
            parameters=_schema(props, req),
            canonical=ToolRef(name=name),
            domains=["filesystem"],
        )
        for name, (desc, props, req) in _SPECS.items()
    ]
    cat = Catalog(runtime=RUNTIME_ID, catalog_version="1", capabilities=caps)
    cat.catalog_digest = cat.computed_digest()
    return cat


class LocalReadOnlyAuthority:
    """RuntimeAuthority over one workspace directory. See the module docstring for its limits."""

    runtime_id = RUNTIME_ID
    policy_engine = POLICY_ENGINE

    def __init__(self, workspace: str, *, fault: Optional[Callable[[CapabilityRequest], None]] = None) -> None:
        root = Path(workspace)
        if not root.is_dir():
            raise ValueError("workspace must be an existing directory")
        self.root = root.resolve()
        self._fault = fault
        self._minted: set = set()
        self._issued: set = set()
        self.decide_calls = 0
        self.execute_calls = 0

    # -- RuntimeAuthority ---------------------------------------------------------------
    def catalog(self) -> Catalog:
        return build_catalog()

    def _decision(self, req, value, reason=None, approval=None) -> Decision:
        return Decision(
            request_id=req.request_id,
            decision=value,
            capability=req.capability,
            authority={"runtime": RUNTIME_ID, "policy_engine": POLICY_ENGINE, "decision_id": None},
            reason=reason,
            constraints=[],
            approval=approval,
        )

    def _confine(self, raw: object, *, must_exist: bool) -> Optional[Path]:
        """The resolved path if it is inside the workspace and not reached through a symlink."""
        if not isinstance(raw, str) or not raw or "\x00" in raw:
            return None
        candidate = Path(raw)
        full = candidate if candidate.is_absolute() else self.root / candidate
        parts = full.relative_to(self.root).parts if full.is_relative_to(self.root) else None
        if parts is None or ".." in Path(raw).parts:
            return None
        walk = self.root
        for part in parts:
            walk = walk / part
            if walk.is_symlink():
                return None
        resolved = full.resolve()
        if resolved != self.root and not resolved.is_relative_to(self.root):
            return None
        if must_exist and not resolved.exists():
            return None
        return resolved

    @staticmethod
    def _shape_problem(args: dict, name: str) -> Optional[str]:
        _desc, props, required = _SPECS[name]
        for key in required:
            if key not in args:
                return f"missing required argument: {key}"
        for key, value in args.items():
            if key not in props:
                return f"unexpected argument: {key}"
            if not isinstance(value, str):
                return f"argument {key} must be string"
        return None

    def decide(self, req: CapabilityRequest, ctx: dict) -> Decision:
        self.decide_calls += 1
        if req.capability not in _SPECS:
            return self._decision(req, "not_found", f"unknown capability: {req.capability}")
        why = self._shape_problem(req.arguments, req.capability)
        if why:
            return self._decision(req, "invalid", why)
        args = req.arguments
        if req.capability == "write_note":
            if len(args["text"]) > MAX_NOTE:
                return self._decision(req, "invalid", "text too long")
            target = self._confine(args["path"], must_exist=False)
            if target is None or target.exists() or not target.parent.is_dir():
                return self._decision(req, "denied", "path outside workspace or already exists")
            self._minted.add(req.request_id)
            approval = {"approval_id": f"local-approval-{req.request_id}", "scope": "single_action",
                        "expires_at": None}
            return self._decision(req, "requires_approval", "a person must approve this write", approval)
        target = self._confine(args["path"], must_exist=True)
        if target is None:
            return self._decision(req, "denied", "path outside workspace or not found")
        if req.capability == "read_file" and not target.is_file():
            return self._decision(req, "denied", "not a regular file")
        if req.capability == "list_dir" and not target.is_dir():
            return self._decision(req, "denied", "not a directory")
        return self._decision(req, "authorized")

    def issue_continuation(self, pending: PendingApproval, *, approved: bool) -> Decision:
        """The authority's own answer to a held request. Issued at most once per request."""
        rid = pending.request_id
        req = pending.capability_request
        if rid not in self._minted or rid in self._issued:
            return self._decision(req, "denied", "no live approval for this request")
        self._issued.add(rid)
        if not approved:
            return self._decision(req, "denied", "approver declined")
        return self._decision(req, "authorized", None,
                              {"approval_id": pending.approval_id, "scope": "single_action", "expires_at": None})

    def execute(self, req: CapabilityRequest, decision: Decision, ctx: dict) -> ToolResult:
        self.execute_calls += 1
        rid, cap = req.request_id, req.capability

        def fail(message, code=ErrorCode.EXECUTION_ERROR, status="error"):
            return make_result(rid, status, runtime=RUNTIME_ID, capability=cap, code=code, message=message)

        if not (decision.is_authorized and decision.request_id == rid and cap in _SPECS):
            return fail(NOT_EXECUTED)
        try:
            if self._fault is not None:
                self._fault(req)
            args = req.arguments
            if cap == "write_note":
                target = self._confine(args["path"], must_exist=False)
                if target is None:
                    return fail("path outside workspace")
                with open(target, "x", encoding="utf-8") as fh:
                    fh.write(args["text"])
                return make_result(rid, "ok", runtime=RUNTIME_ID, capability=cap,
                                   data={"path": args["path"], "bytes": len(args["text"].encode("utf-8"))},
                                   content_kind="tool_result", trust="trusted_runtime")
            target = self._confine(args["path"], must_exist=True)
            if target is None:
                return fail("path outside workspace or not found")
            if cap == "list_dir":
                names = sorted(os.listdir(target))
                return make_result(rid, "ok", runtime=RUNTIME_ID, capability=cap,
                                   data={"path": args["path"], "entries": names},
                                   content_kind="workspace_content", trust="workspace_untrusted")
            size = target.stat().st_size
            if size > MAX_READ:
                return fail("file too large")
            text = target.read_text(encoding="utf-8")
            return make_result(rid, "ok", runtime=RUNTIME_ID, capability=cap,
                               data={"path": args["path"], "content": text},
                               content_kind="workspace_content", trust="workspace_untrusted")
        except TimeoutError:
            return fail("execution exceeded its deadline", ErrorCode.EXECUTION_TIMEOUT, "timed_out")
        except UncertainEffect:
            return fail("uncertain effect: the response was lost and the effect may have happened")
        except (OSError, UnicodeDecodeError):
            return fail("execution failed")
