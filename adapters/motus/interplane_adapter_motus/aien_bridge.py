"""AienBridgeAuthority: the REAL AIEN authority, reached through a child process.

``adapters/aien`` (Rust) wraps AIEN's own ``AienAuthority``. Its binary ``aien-authority-bridge``
speaks newline-delimited JSON on stdin and stdout. This class spawns that binary and is a
``RuntimeAuthority`` for the Motus gate. It decides nothing:

* ``decide`` and ``execute`` are forwarded one line each; the answers are parsed with the
  ``interplane.core`` types (``Decision``, ``ToolResult``, ``Catalog``), no schema of its own.
* Any fault (binary missing, would not start, malformed JSON, wrong request id, timeout, exit in
  the middle of a call) fails closed: ``decide`` returns ``denied`` with reason
  ``aien bridge unavailable`` and the bridge is never used again in this run. ``execute`` is
  never reached for a denied request. A lost ``execute`` is reported as an uncertain effect.
"""

from __future__ import annotations

import json
import os
import select
import subprocess
import threading
import time
from typing import Any, Optional

from interplane.core import Catalog, CapabilityRequest, Decision, ErrorCode, ToolResult
from interplane.crossaxis import MappingTable
from interplane.crossveil import PendingApproval, make_result

from .gate import PRE_EXECUTE_UNAVAILABLE

ENV_BRIDGE = "INTERPLANE_AIEN_BRIDGE"
UNAVAILABLE = "aien bridge unavailable"
POLICY_ENGINE = "interplane-adapter-motus.aien-bridge.fail_closed"
DEFAULT_TIMEOUT = 10.0
MAX_LINE = 4 * 1024 * 1024
# The bridge needs no secrets: only a way to find libraries, a home, a locale and quiet backtraces.
_ENV_KEEP = ("PATH", "HOME", "LANG", "LC_ALL", "TMPDIR")


def minimal_env() -> dict:
    env = {k: os.environ[k] for k in _ENV_KEEP if k in os.environ}
    env["RUST_BACKTRACE"] = "0"
    return env


class BridgeFault(Exception):
    """The bridge could not give a trustworthy answer. Always ends in a fail-closed result."""


class BridgeUnavailable(BridgeFault):
    """The bridge was gone before any byte of the request was written: nothing was sent."""


def _plain(value: Any) -> Any:
    return value.to_dict() if hasattr(value, "to_dict") else value


class AienBridgeAuthority:
    """RuntimeAuthority over ``aien-authority-bridge``. See the module docstring."""

    policy_engine = POLICY_ENGINE

    def __init__(
        self,
        workspace: str,
        *,
        bridge_path: Optional[str] = None,
        timeout: float = DEFAULT_TIMEOUT,
    ) -> None:
        self.workspace = workspace
        self.timeout = timeout
        self.decide_calls = 0
        self.execute_calls = 0
        self.fault: Optional[str] = None
        self.runtime_id = "aien"
        self._lock = threading.Lock()
        self._buf = b""
        self._proc: Optional[subprocess.Popen] = None
        self._catalog: Optional[Catalog] = None
        path = bridge_path or os.environ.get(ENV_BRIDGE)
        if not path:
            self._fail("no bridge binary configured")
            return
        try:
            self._proc = subprocess.Popen(
                [path, workspace],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                env=minimal_env(),
            )
            os.set_blocking(self._proc.stdin.fileno(), False)
        except (OSError, ValueError) as err:
            self._fail(f"cannot start bridge: {err.__class__.__name__}")
            return
        try:
            cat = Catalog.from_dict(self._rpc({"op": "catalog"}, "catalog"))
            if not cat.runtime:
                raise BridgeFault("catalog names no runtime")
            self._catalog = cat
            self.runtime_id = cat.runtime
        except Exception as err:  # noqa: BLE001 - any fault at start means no authority
            self._fail(f"bad catalog: {err}")

    # -- process and wire ---------------------------------------------------------------
    @property
    def available(self) -> bool:
        return self.fault is None and self._proc is not None

    def _fail(self, why: str) -> None:
        self.fault = self.fault or why
        proc, self._proc = self._proc, None
        if proc is not None:
            try:
                proc.kill()
            except OSError:
                pass
            try:
                proc.wait(timeout=2)
            except Exception:  # noqa: BLE001
                pass

    def close(self) -> None:
        with self._lock:
            proc, self._proc = self._proc, None
            if proc is None:
                return
            try:
                proc.stdin.close()
                proc.wait(timeout=2)
            except Exception:  # noqa: BLE001
                proc.kill()
                proc.wait()

    def _readline(self, deadline: float) -> bytes:
        fd = self._proc.stdout.fileno()
        while b"\n" not in self._buf:
            left = deadline - time.monotonic()
            if left <= 0:
                raise BridgeFault("timeout")
            ready, _, _ = select.select([fd], [], [], left)
            if not ready:
                raise BridgeFault("timeout")
            chunk = os.read(fd, 65536)
            if not chunk:
                raise BridgeFault("bridge exited")
            self._buf += chunk
            if len(self._buf) > MAX_LINE and b"\n" not in self._buf:
                raise BridgeFault("response too large")
        line, _, self._buf = self._buf.partition(b"\n")
        return line

    def _write(self, data: bytes, deadline: float) -> None:
        """Write the whole request before the deadline; a bridge that stops reading cannot hang the host."""
        fd = self._proc.stdin.fileno()
        view = memoryview(data)
        total = len(view)
        while view:
            left = deadline - time.monotonic()
            if left <= 0:
                raise BridgeFault("timeout writing to bridge")
            _, writable, _ = select.select([], [fd], [], left)
            if not writable:
                raise BridgeFault("timeout writing to bridge")
            try:
                view = view[os.write(fd, view[:65536]):]
            except BlockingIOError:
                continue
            except BrokenPipeError as err:
                if len(view) == total:
                    raise BridgeUnavailable("bridge closed its input") from err
                raise

    def _rpc(self, message: dict, key: str, request_id: Optional[str] = None) -> dict:
        """One request line, one response line. Raises BridgeFault on anything but a clean answer."""
        if self._proc is None:
            raise BridgeUnavailable(self.fault or "bridge not running")
        if self._proc.poll() is not None:
            raise BridgeUnavailable("bridge exited")
        deadline = time.monotonic() + self.timeout
        try:
            self._write(json.dumps(message, separators=(",", ":")).encode("utf-8") + b"\n", deadline)
            line = self._readline(deadline)
            self._no_stray_bytes()
        except (BrokenPipeError, OSError, ValueError) as err:
            raise BridgeFault(f"bridge io: {err.__class__.__name__}") from err
        try:
            resp = json.loads(line.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as err:
            raise BridgeFault("malformed json from bridge") from err
        if not isinstance(resp, dict) or resp.get("ok") is not True or not isinstance(resp.get(key), dict):
            raise BridgeFault("bridge refused or returned an unexpected shape")
        if request_id is not None and resp[key].get("request_id") != request_id:
            raise BridgeFault("response is for a different request")
        return resp[key]

    def _no_stray_bytes(self) -> None:
        """One request, one line. Anything already buffered or waiting on the pipe is a framing fault."""
        if self._buf:
            raise BridgeFault("unexpected extra output from bridge")
        ready, _, _ = select.select([self._proc.stdout.fileno()], [], [], 0)
        if ready:
            raise BridgeFault("unexpected extra output from bridge")

    def _call(self, message: dict, key: str, request_id: Optional[str] = None) -> dict:
        with self._lock:
            try:
                return self._rpc(message, key, request_id)
            except BridgeFault as err:
                self._fail(str(err))
                raise

    # -- RuntimeAuthority ---------------------------------------------------------------
    def catalog(self) -> Catalog:
        if self._catalog is not None:
            return self._catalog
        empty = Catalog(runtime=self.runtime_id, catalog_version="0", capabilities=[])
        empty.catalog_digest = empty.computed_digest()
        return empty

    def mapping_table(self) -> MappingTable:
        """AIEN's own mapping table for its catalog (pinned to the catalog digest)."""
        return MappingTable.from_dict(self._call({"op": "mapping_table"}, "mapping_table"))

    def _denied(self, req: CapabilityRequest) -> Decision:
        return Decision(
            request_id=req.request_id,
            decision="denied",
            capability=req.capability,
            authority={"runtime": self.runtime_id, "policy_engine": POLICY_ENGINE, "decision_id": None},
            reason=UNAVAILABLE,
            constraints=[],
            approval=None,
        )

    @staticmethod
    def _ctx(ctx: dict) -> dict:
        return {k: _plain(v) for k, v in (ctx or {}).items()}

    def _checked_decision(self, raw: dict, req: CapabilityRequest) -> Decision:
        decision = Decision.from_dict(raw)
        if decision.request_id != req.request_id or decision.capability != req.capability:
            raise BridgeFault("decision is for a different request")
        return decision

    def decide(self, req: CapabilityRequest, ctx: dict) -> Decision:
        self.decide_calls += 1
        if not self.available:
            return self._denied(req)
        try:
            raw = self._call(
                {"op": "decide", "request": req.to_dict(), "ctx": self._ctx(ctx)}, "decision",
                req.request_id,
            )
            return self._checked_decision(raw, req)
        except Exception as err:  # noqa: BLE001 - fail closed on anything at all
            self._fail(f"decide: {err}")
            return self._denied(req)

    def execute(self, req: CapabilityRequest, decision: Decision, ctx: dict) -> ToolResult:
        self.execute_calls += 1
        rid = req.request_id
        if not self.available:
            return make_result(rid, "error", runtime=self.runtime_id, capability=req.capability,
                               code=ErrorCode.RUNTIME_UNAVAILABLE, message=PRE_EXECUTE_UNAVAILABLE)
        try:
            raw = self._call(
                {"op": "execute", "request": req.to_dict(), "decision": decision.to_dict(),
                 "ctx": self._ctx(ctx)},
                "result",
                rid,
            )
            result = ToolResult.from_dict(raw)
            if result.request_id != rid:
                raise BridgeFault("result is for a different request")
            return result
        except BridgeUnavailable as err:
            self._fail(f"execute: {err}")
            return make_result(rid, "error", runtime=self.runtime_id, capability=req.capability,
                               code=ErrorCode.RUNTIME_UNAVAILABLE, message=PRE_EXECUTE_UNAVAILABLE)
        except Exception as err:  # noqa: BLE001
            self._fail(f"execute: {err}")
            return make_result(
                rid, "error", runtime=self.runtime_id, capability=req.capability,
                code=ErrorCode.EXECUTION_ERROR,
                message="uncertain effect: the bridge failed during execute and the effect may have happened",
            )

    def issue_continuation(self, pending: PendingApproval, *, approved: bool) -> Decision:
        """AIEN's own answer to a held request: its desk issues one single-use grant for exactly
        this request and AIEN spends it. Declining is ``Gate.cancel_approval``, not a decision made here."""
        req = pending.capability_request
        if not approved:
            raise BridgeFault("decline with Gate.cancel_approval; this adapter issues no decisions")
        raw = self._call({"op": "approve", "request": req.to_dict()}, "decision", req.request_id)
        return self._checked_decision(raw, req)
