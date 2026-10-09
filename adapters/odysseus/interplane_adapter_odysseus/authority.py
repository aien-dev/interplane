"""OdysseusAuthority: a Crossveil runtime adapter that translates, and never decides.

Every ``decide`` outcome below comes from Odysseus's own code (cited per branch). This module only
maps Odysseus's answers onto INTERPLANE's five decision values. If Odysseus cannot be imported the
adapter fails closed (``denied``); it never authorizes on its own.

Reused from Odysseus (commit a8c147b), by call; line numbers are at a8c147b:
  * ``src.agent_tools.TOOL_TAGS``                       name known?       (tool_schemas.py:2161 rule)
  * ``src.tool_schemas.function_call_to_tool_block``    argument validity (tool_schemas.py:2048)
  * ``src.tool_capabilities.ToolRunSecurityContext.decision_for`` + ``observe_tool_result`` +
    ``capabilities_for_action``                         untrusted-context gate (tool_capabilities.py:818-876, :878)
  * ``src.tool_security.NON_ADMIN_BLOCKED_TOOLS`` / ``is_public_blocked_tool`` and
    ``src.tool_execution._ADMIN_TOOLS``                 admin gates (tool_execution.py:2032-2036, :2060-2074)
  * ``src.tool_execution._resolve_tool_path_in_workspace`` / ``vet_workspace`` /
    ``_active_workspace`` + ``src.agent_tools.TOOL_HANDLERS`` (ReadFileTool, LsTool, GlobTool,
    GrepTool)                                           confinement and execution
Used by the launch preflight (see launch.py):
  * ``src.agent_runtime.process_resources.guard_launch_workspace`` (:379) with
    ``src.agent_runtime.resources.FilesystemRoot.seal`` / ``ResourceIdentityError``,
    ``src.constants.DATA_DIR`` and ``src.tool_execution.vet_workspace`` (read-only, structural).
Gate order follows ``execute_tool_block`` (tool_execution.py:1858-1869 then :2032-2074): the
untrusted-context gate first, then the admin gates, then (in Odysseus: inside the tool) path
confinement. Confinement is also run in ``decide`` so that an escape is refused BEFORE execution.
"""

from __future__ import annotations

import asyncio
import copy
import json
import logging
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from typing import Any, Optional

from interplane.core import Catalog, CapabilityRequest, Decision, ToolResult
from interplane.crossveil import RuntimeAuthority, make_result

from . import _odysseus
from .launch import launch_preflight
from .domains import catalog_with_domains

RUNTIME_ID = "odysseus"
POLICY_ENGINE = "odysseus.tool_capabilities"
NOT_AVAILABLE = "odysseus runtime not available"
NOT_EXECUTED = "not executed by the reference adapter"
APPROVAL_CONTINUATION = "approval continuation unsupported on Odysseus"
EXECUTABLE = frozenset({"read_file", "ls", "glob", "grep"})
_PATH_TOOLS = EXECUTABLE
_UNTRUSTED = frozenset({"external_untrusted", "workspace_untrusted"})
_TRUSTED_FLOORS = frozenset({"trusted_runtime", "user_supplied"})
_MAX_REASON = 4096


class OdysseusAuthority(RuntimeAuthority):
    """Translate Odysseus's authority into Crossveil decisions; execute four read-only tools.

    Args:
      workspace: root the read-only tools are confined to (None: those tools are refused).
      admin: whether the caller counts as an Odysseus admin. Default False (fail closed): the
        real check, ``owner_is_admin_or_single_user``, needs Odysseus's auth state, which the
        adapter does not construct. Pass the answer your Odysseus deployment gives.
      workspace_trusted: label file reads ``trusted_runtime`` instead of Odysseus's own
        ``workspace_untrusted``.
      external_context_seen: start every trace with the untrusted-context gate already armed.
      delegated_credential: the caller is an API-token client (Odysseus refuses privileged tools).
      odysseus: an already loaded ``_odysseus.Odysseus`` (default: ``_odysseus.load()``; pass
        ``False`` to force the not-available path).
    """

    runtime_id = RUNTIME_ID
    policy_engine = POLICY_ENGINE

    def __init__(
        self,
        workspace: Optional[str] = None,
        *,
        admin: bool = False,
        workspace_trusted: bool = False,
        external_context_seen: bool = False,
        delegated_credential: bool = False,
        odysseus: Any = None,
    ) -> None:
        if odysseus is False:
            self._ody = None
        else:
            self._ody = odysseus if odysseus is not None else _odysseus.load()
        self.admin = admin
        self.workspace_trusted = workspace_trusted
        self.external_context_seen = external_context_seen
        self.delegated_credential = delegated_credential
        self.workspace: Optional[str] = None
        if workspace is not None and self._ody is not None:
            self.workspace = self._ody.tool_execution.vet_workspace(workspace)
            if self.workspace is None:
                raise ValueError("workspace is not a usable directory by Odysseus's own rules")
        self._catalog = catalog_with_domains()
        self._contexts: dict = {}
        self._lock = threading.Lock()
        self.decisions: dict = {}  # request_id -> last Decision (observability, never reused)

    # -- catalog --------------------------------------------------------------------------------

    def catalog(self) -> Catalog:
        return copy.deepcopy(self._catalog)

    @property
    def available(self) -> bool:
        return self._ody is not None

    def launch_preflight(self, workspace: Optional[str] = None, tool: str = "bash"):
        """Structural process-launch check (see ``launch.py``); never authorizes or executes."""
        return launch_preflight(self._ody, self.workspace if workspace is None else workspace, tool)

    # -- helpers --------------------------------------------------------------------------------

    def _decision(
        self,
        req: CapabilityRequest,
        value: str,
        reason: Optional[str] = None,
        approval: Optional[dict] = None,
        runtime_state: Optional[dict] = None,
    ) -> Decision:
        if reason is not None:
            reason = self._redact(reason)[:_MAX_REASON]
        return Decision(
            request_id=req.request_id,
            decision=value,
            capability=req.capability,
            authority={"runtime": RUNTIME_ID, "policy_engine": POLICY_ENGINE, "decision_id": None},
            reason=reason,
            approval=approval,
            runtime_state=runtime_state,
        )

    def _redact(self, text: str) -> str:
        """Decision reasons must not leak local paths: show the workspace as ``<workspace>``."""
        if self.workspace:
            text = text.replace(self.workspace, "<workspace>")
        return text

    def _context_for(self, trace_id: str, ctx: dict):
        """One Odysseus ToolRunSecurityContext per trace, armed from the pipeline's exposure."""
        tc = self._ody.tool_capabilities
        with self._lock:
            sc = self._contexts.get(trace_id)
            if sc is None:
                sc = tc.ToolRunSecurityContext(
                    external_untrusted_context_seen=self.external_context_seen,
                    delegated_credential=self.delegated_credential,
                )
                self._contexts[trace_id] = sc
            if _exposure_untrusted(ctx):
                sc.external_untrusted_context_seen = True  # monotonic: never disarmed
            return sc

    def forget_trace(self, trace_id: str) -> None:
        with self._lock:
            self._contexts.pop(trace_id, None)

    def _block(self, name: str, args: dict):
        """Odysseus's own validity check. Returns ``(block, message)``."""
        schemas = self._ody.tool_schemas
        records: list = []
        handler = _Capture(records)
        log = logging.getLogger(schemas.__name__)
        with self._lock:
            log.addHandler(handler)
            try:
                try:
                    block = schemas.function_call_to_tool_block(name, json.dumps(args))
                except (ValueError, TypeError) as err:
                    return None, str(err) or "odysseus rejected the tool call"
            finally:
                log.removeHandler(handler)
        if block is None:
            return None, (records[-1] if records else "odysseus rejected the tool call")
        return block, None

    # -- Crossveil: decide ------------------------------------------------------------------------

    def decide(self, req: CapabilityRequest, ctx: dict) -> Decision:
        decision = self._decide(req, ctx or {})
        self.decisions[req.request_id] = decision
        return decision

    def _decide(self, req: CapabilityRequest, ctx: dict) -> Decision:
        if self._ody is None:
            return self._decision(req, "denied", NOT_AVAILABLE)
        ody = self._ody
        name = req.capability
        if name not in ody.tool_tags:
            return self._decision(req, "not_found", f"unknown capability: {name}")
        block, message = self._block(name, req.arguments)
        if block is None:
            return self._decision(req, "invalid", message)

        # 1. untrusted-context gate (tool_execution.py:1858-1869; tool_capabilities.py:818-876)
        sc = self._context_for(str(ctx.get("trace_id")), ctx)
        gate = sc.decision_for(name, block.content)
        if not gate.allowed and self.delegated_credential and ody.tool_security.is_public_blocked_tool(name):
            # tool_capabilities.py:782-784, :822-829: "no approval can lift that"; the agent loop also adds
            # these tools to its policy blocklist (agent_loop.py:21075, :21279). A refusal, not a prompt.
            return self._denied(req, "delegated_credential", gate.reason or "refused for API-token callers")
        if not gate.allowed:
            reason = gate.reason or "tool blocked by the untrusted-context gate"
            return self._decision(
                req,
                "requires_approval",
                reason,
                # No approval id is minted: the pipeline keeps no pending entry for a request whose
                # requires_approval carries none, so every host continuation is refused
                # (no_pending_approval) and nothing can execute through an approval. Odysseus's
                # sealed approvals are task or chat scoped and bound to owner, session and run, which
                # INTERPLANE's single-effect continuation cannot express (see the README).
                approval=None,
                runtime_state={
                    "vocabulary": "odysseus.tool_gate",
                    "values": [reason, APPROVAL_CONTINUATION],
                },
            )

        # 2. admin gates (tool_execution.py:2032-2036, :2060-2074)
        if not self.admin:
            if name in ody.tool_execution._ADMIN_TOOLS:
                return self._denied(req, "admin_tool", f"Tool '{name}' requires an admin user.")
            if ody.tool_security.is_public_blocked_tool(name):
                return self._denied(
                    req,
                    "non_admin_blocked_tool",
                    f"Tool '{name}' is restricted to admin users on this deployment.",
                )

        # 3. workspace confinement (Odysseus runs it inside the tool; we also run it here)
        if name in _PATH_TOOLS:
            refusal = self._confine(name, req.arguments)
            if refusal is not None:
                return self._denied(req, "workspace_confinement", refusal)

        effects = sorted(
            e.value for e in ody.tool_capabilities.capabilities_for_action(name, block.content).effects
        )
        return self._decision(
            req,
            "authorized",
            runtime_state={"vocabulary": "odysseus.tool_effect", "values": effects},
        )

    def _denied(self, req: CapabilityRequest, why: str, reason: str) -> Decision:
        return self._decision(
            req,
            "denied",
            reason,
            runtime_state={"vocabulary": "odysseus.tool_policy", "values": [why]},
        )

    def _confine(self, name: str, args: dict) -> Optional[str]:
        """None when Odysseus's own resolver accepts the path; else its refusal message."""
        if self.workspace is None:
            return "no workspace configured for this adapter"
        raw = args.get("path")
        raw = raw if isinstance(raw, str) else ""
        if name != "read_file":
            raw = raw.strip() or self.workspace  # same default as _resolve_search_root (tool_execution.py:1237)
        try:
            self._ody.tool_execution._resolve_tool_path_in_workspace(self.workspace, raw)
        except ValueError as err:
            return str(err)
        return None

    # -- Crossveil: execute -----------------------------------------------------------------------

    def execute(self, req: CapabilityRequest, decision: Decision, ctx: dict) -> ToolResult:
        name, rid = req.capability, req.request_id
        if decision.decision != "authorized":  # fail closed: only an authorized decision executes
            return self._error(rid, name, NOT_EXECUTED)
        if self._ody is None:
            return self._error(rid, name, NOT_AVAILABLE)
        if name not in EXECUTABLE:
            return self._error(rid, name, NOT_EXECUTED)
        if self.workspace is None:
            return self._error(rid, name, "no workspace configured for this adapter")
        block, message = self._block(name, req.arguments)
        if block is None:
            return self._error(rid, name, message or "odysseus rejected the tool call")
        started = time.monotonic()
        try:
            raw = self._run_handler(name, block.content)
        except Exception as err:  # noqa: BLE001 - reported as an execution error, not raised
            return self._error(rid, name, f"{name}: {type(err).__name__}")
        ms = int((time.monotonic() - started) * 1000)
        trust = "trusted_runtime" if self.workspace_trusted else self._integrity(name, block.content, raw)
        sc = self._context_for(str(ctx.get("trace_id")), {})
        sc.observe_tool_result(name, raw, block.content)  # Odysseus decides what arms the gate
        ok = isinstance(raw, dict) and not raw.get("error") and raw.get("exit_code") in (None, 0)
        if ok:
            return make_result(
                rid, "ok", runtime=RUNTIME_ID, capability=name, data=raw,
                content_kind="workspace_content", trust=trust, duration_ms=ms,
            )
        text = str(raw.get("error")) if isinstance(raw, dict) and raw.get("error") else "execution failed"
        return make_result(
            rid, "error", runtime=RUNTIME_ID, capability=name, code="execution_error",
            message=self._redact(text)[:_MAX_REASON], content_kind="workspace_content",
            trust=trust, duration_ms=ms,
        )

    def _integrity(self, name: str, content: str, raw: Any = None) -> str:
        """Odysseus's result integrity as a TrustLevel.

        ``workspace_untrusted`` and ``external_untrusted`` carry over. ``system`` is Odysseus's
        label for server-authored output and its default for a registered tool
        (tool_capabilities.py:39-42 at a8c147b): it maps to ``trusted_runtime``, unless
        Odysseus's own ``tool_result_should_arm_gate`` (:619-664) says this result carries
        non-system content (the producer set ``untrusted_content``), then ``external_untrusted``.
        Any other value is ``external_untrusted``.
        """
        tc = self._ody.tool_capabilities
        value = tc.capabilities_for_action(name, content).result_integrity.value
        if value == "system":
            if raw is not None and tc.tool_result_should_arm_gate(name, raw, content):
                return "external_untrusted"
            return "trusted_runtime"
        return value if value in _UNTRUSTED else "external_untrusted"

    def _error(self, rid: str, name: str, message: str) -> ToolResult:
        return make_result(
            rid, "error", runtime=RUNTIME_ID, capability=name, code="execution_error",
            message=message, content_kind="tool_result", trust="trusted_runtime",
        )

    def _run_handler(self, name: str, content: str) -> dict:
        """Run Odysseus's own tool handler with the workspace bound the way Odysseus binds it."""
        te = self._ody.tool_execution
        handler = self._ody.tool_handlers[name]
        workspace = self.workspace

        async def run() -> dict:
            token = te._active_workspace.set(workspace)
            try:
                return await handler(content, {"progress_cb": None, "session_id": None, "owner": None})
            finally:
                te._active_workspace.reset(token)

        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(run())
        with ThreadPoolExecutor(max_workers=1) as pool:  # called from inside a running loop
            return pool.submit(lambda: asyncio.run(run())).result()


class _Capture(logging.Handler):
    def __init__(self, sink: list) -> None:
        super().__init__(level=logging.WARNING)
        self.sink = sink

    def emit(self, record: logging.LogRecord) -> None:
        self.sink.append(record.getMessage())


def _exposure_untrusted(ctx: dict) -> bool:
    """True when the pipeline's exposure says the model could see untrusted content (0.3 cut E2).

    ``ctx["exposure"]`` is computed by the pipeline from its own input ledger (CROSSVEIL.md Trust
    rules 5 and 6), never by the model or the caller. A floor below ``user_supplied`` arms the gate,
    as Odysseus arms it natively after any non-system tool result (tool_capabilities.py:619-664,
    878-887). A missing or malformed exposure arms it too (fail closed).
    """
    exposure = ctx.get("exposure")
    if not isinstance(exposure, dict):
        return True
    return exposure.get("floor") not in _TRUSTED_FLOORS
