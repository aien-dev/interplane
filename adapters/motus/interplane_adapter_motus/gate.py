"""The gate: one Motus agent run, one INTERPLANE trace.

``Gate.call`` takes what Motus hands a tool (a tool name and the raw JSON argument string),
runs it through the real INTERPLANE ``Pipeline`` against a ``RuntimeAuthority`` and returns the
text Motus puts in front of the model. It never decides anything:

* ``decide`` and ``execute`` are reached only through the authority the host supplied.
* Nothing here builds a ``Decision`` or a ``ToolResult``. A continuation decision comes from an
  ``approver`` callable that belongs to the authority side.
* Every failure inside the adapter ends in a refusal record with ``executed: false``.
* There is no retry anywhere. A timeout, a cancellation or an uncertain effect is reported once.
"""

from __future__ import annotations

import copy
import threading
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Callable, Optional

from interplane.core import Catalog, Decision, ErrorCode, digest, jcs, text_digest
from interplane.crossaxis import MappingTable
from interplane.crossveil import (
    ContinuationRefused,
    PendingApproval,
    Pipeline,
    capability_request_digest,
)

from . import MOTUS_PIN
from .evidence import UNAVAILABLE_MOTUS_ID, build_bundle
from .ids import new_trace_id, valid_id, valid_trace_id

# The authority was already gone before the execute request was written: nothing was sent, so no effect.
PRE_EXECUTE_UNAVAILABLE = "aien bridge unavailable before execute; no request was sent"
_MOTUS_ID_KEYS = ("task_id", "run_id", "tool_call_id")

# Outcome vocabulary (contract C2, lower case) and effect certainty.
FINISHED, REJECTED, UNAVAILABLE = "finished", "rejected", "unavailable"
TIMEOUT, CANCELLED, UNCERTAIN = "timeout", "cancelled", "uncertain"
FAILED, APPROVAL_PENDING = "failed", "approval_pending"


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class RecordingAuthority:
    """Pass-through proxy: forwards decide/execute/catalog unchanged and remembers digests.

    It cannot change an answer. It exists so the evidence can quote the digest of the exact
    capability request and decision the authority saw, and count execute calls independently of
    the Pipeline's own log.
    """

    def __init__(self, inner: Any) -> None:
        self._inner = inner
        self.runtime_id = inner.runtime_id
        self.seen: dict = {}

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def _entry(self, rid: str) -> dict:
        return self.seen.setdefault(
            rid,
            {"request_digest": None, "decision_digest": None, "decide_calls": 0, "execute_calls": 0},
        )

    def decide(self, req, ctx):
        entry = self._entry(req.request_id)
        entry["decide_calls"] += 1
        entry["request_digest"] = capability_request_digest(req)
        raw = self._inner.decide(req, ctx)
        try:
            entry["decision_digest"] = digest(raw.to_dict() if hasattr(raw, "to_dict") else raw)
        except Exception:  # noqa: BLE001 - an undigestable answer is left unlabelled, not fixed
            entry["decision_digest"] = None
        return raw

    def execute(self, req, decision, ctx):
        self._entry(req.request_id)["execute_calls"] += 1
        return self._inner.execute(req, decision, ctx)

    def catalog(self) -> Catalog:
        return self._inner.catalog()


@dataclass
class CallOutcome:
    """What one call produced: the text Motus shows the model and the evidence record."""

    text: str
    record: dict

    @property
    def outcome(self) -> str:
        return self.record["outcome"]

    @property
    def executed(self) -> bool:
        return self.record["executed"]


def _refusal_text(code: str, message: str, status: str = "rejected") -> str:
    return jcs({"error": {"code": code, "message": message}, "status": status})


def classify(status: Optional[str], code: Optional[str], message: str, executed: bool) -> tuple:
    """(outcome, effect_certainty) from the Pipeline's own result. Label only, never a grant."""
    if status == "ok":
        return FINISHED, "occurred"
    if status == "timed_out":
        return TIMEOUT, "uncertain"
    if status == "error" and executed:
        if code == ErrorCode.RUNTIME_UNAVAILABLE and message == PRE_EXECUTE_UNAVAILABLE:
            return UNAVAILABLE, "none"  # our own exact marker: the request was never written
        # After the execute request went out, only an explicit ok result says the effect occurred.
        return UNCERTAIN, "uncertain"
    if status == "requires_approval":
        return APPROVAL_PENDING, "none"
    if code == ErrorCode.RUNTIME_UNAVAILABLE:
        return UNAVAILABLE, "none"
    return REJECTED, "none"


class Gate:
    """Authorize Motus tool calls through INTERPLANE and keep the evidence for one run."""

    def __init__(
        self,
        authority: Any,
        mapping_table: MappingTable,
        *,
        trace_id: Optional[str] = None,
        model: str = "motus",
        motus_run_id: Optional[str] = None,
        motus_installed_version: Optional[str] = None,
        pipeline_factory: Callable[..., Pipeline] = Pipeline,
        clock: Callable[[], str] = utc_now,
    ) -> None:
        if trace_id is None:
            trace_id = new_trace_id()
        if not valid_trace_id(trace_id):
            raise ValueError("trace_id must be 32 lowercase hex characters")
        self.trace_id = trace_id
        self.model = model
        self.authority = RecordingAuthority(authority)
        self.pipeline = pipeline_factory(None, mapping_table, self.authority)
        self._clock = clock
        self._lock = threading.RLock()
        self._turn = 0
        self._cancelled: set = set()
        self._records: list = []
        self._continued: set = set()  # request ids for which an approver was already asked
        self._calls: dict = {}  # request_id -> the call record (for continuations)
        self._closed = False
        self._motus_run_id = motus_run_id
        self._motus_version = motus_installed_version
        self._catalog_at_start = self._catalog_digest()

    # -- catalog exposure ----------------------------------------------------------
    def _catalog_digest(self) -> Optional[str]:
        try:
            cat = self.authority.catalog()
            return cat.catalog_digest or cat.computed_digest()
        except Exception:  # noqa: BLE001 - label it unavailable instead of guessing
            return None

    def exposed_tools(self) -> list:
        """What the model may be shown: the authority's own catalog, nothing added."""
        cat = self.authority.catalog()
        return [
            {"name": c.name, "description": c.description, "parameters": copy.deepcopy(c.parameters)}
            for c in cat.capabilities
        ]

    # -- identifiers ----------------------------------------------------------------
    def new_call_id(self) -> str:
        with self._lock:
            self._turn += 1
            return f"{self.trace_id}-{self._turn:06d}"

    def note_cancellation(self, call_id: str) -> None:
        """Motus cancelled the call. The call is never retried; its record says cancelled."""
        self._cancelled.add(call_id)  # no lock on purpose: the worker may hold it mid-execute

    def register_user_turn(self, text: str, input_id: str = "in-user-1") -> None:
        """Record the user's request so the exposure floor starts at user_supplied."""
        self.pipeline.register_input(
            {
                "input_id": input_id,
                "content_kind": "user_request",
                "trust": "user_supplied",
                "source": {"kind": "operator", "id": "user"},
                "origin": "runtime:user-turn-1",
                "content_digest": text_digest(text),
                "trace_id": self.trace_id,
                "parent_id": None,
                "derived_from": [],
            }
        )

    # -- the call -------------------------------------------------------------------
    def call(
        self,
        tool_name: Any,
        raw_arguments: Any,
        *,
        call_id: Optional[str] = None,
        motus_ids: Optional[dict] = None,
    ) -> CallOutcome:
        with self._lock:
            return self._call(tool_name, raw_arguments, call_id, motus_ids)

    def _call(self, tool_name, raw_arguments, call_id, motus_ids) -> CallOutcome:
        if call_id is None:
            self._turn += 1
            call_id = f"{self.trace_id}-{self._turn:06d}"
        base = self._base_record("call", call_id, tool_name, motus_ids)
        if self._closed:
            return self._refuse(base, "session_closed", "gate is closed")
        if not valid_id(call_id):
            return self._refuse(base, ErrorCode.MALFORMED_TOOL_CALL, "call id is not a valid request id")
        # A repeated id is not refused here: the Pipeline ledger rejects it and the record shows that.
        if not isinstance(tool_name, str) or not isinstance(raw_arguments, str):
            return self._refuse(base, ErrorCode.MALFORMED_TOOL_CALL, "tool name and arguments must be strings")
        if call_id in self._cancelled:
            rec = dict(base, outcome=CANCELLED, effect_certainty="none", status="cancelled",
                       error_code=None, cancelled=True)
            return self._finish(rec, _refusal_text("cancelled", "cancelled before dispatch", "cancelled"))
        self._turn += 1
        turn = self._turn
        prefix = f"motus-{self.trace_id}-{turn}"
        payload = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": call_id,
                    "type": "function",
                    "function": {"name": tool_name, "arguments": raw_arguments},
                }
            ],
        }
        base["message_id"] = f"{prefix}-0"
        base["arguments_digest"] = text_digest(raw_arguments)
        try:
            out = self.pipeline.run_turn(
                "openai", self.model, payload, self.trace_id, turn, message_id_prefix=prefix
            )
            result, observed = out.results[0], out.observed[0]
            text = out.rendered[0]["content"]
        except Exception:  # noqa: BLE001 - fail closed, no retry
            return self._refuse(base, ErrorCode.RUNTIME_UNAVAILABLE, "pipeline fault; nothing was retried")
        return self._from_pipeline(base, call_id, result, observed, text)

    def _from_pipeline(self, base, call_id, result, observed, text) -> CallOutcome:
        rid = base["request_id"]
        seen = self.authority.seen.get(rid, {}) if observed.decide_invoked else {}
        message = result.error.message if result.error else ""
        code = result.error.code if result.error else None
        outcome, certainty = classify(result.status, code, message, observed.execute_invoked)
        cancelled = call_id in self._cancelled
        if cancelled:
            outcome = CANCELLED
            certainty = "uncertain" if observed.execute_invoked else "none"
            text = _refusal_text("cancelled", "cancelled; result withheld and not retried", "cancelled")
        approval_id = None
        if result.decision is not None and result.decision.approval:
            approval_id = result.decision.approval_id
        rec = dict(
            base,
            capability_request_digest=seen.get("request_digest"),
            decision=observed.decision,
            decision_digest=seen.get("decision_digest"),
            approval_id=approval_id,
            stage=observed.stage,
            status=result.status,
            error_code=code,
            executed=bool(observed.execute_invoked),
            authority_execute_calls=seen.get("execute_calls", 0),
            result_digest=observed.result_digest,
            model_visible_digest=text_digest(text),
            outcome=outcome,
            effect_certainty=certainty,
            cancelled=cancelled,
        )
        if rec["capability_request_digest"] is None:
            rec["unavailable"] = "request was refused before the authority was asked (no capability request exists)"
        return self._finish(rec, text)

    # -- approval continuation --------------------------------------------------------
    def approve(
        self,
        request_id: str,
        approver: Callable[[PendingApproval], Any],
        *,
        now: Optional[str] = None,
    ) -> CallOutcome:
        """Continue a held request once. ``approver`` is the authority side: it returns the
        runtime's own continuation decision for the pending approval it is shown. The adapter
        forwards it and binds it to the exact request digest recorded when the authority was
        asked; the Pipeline then re-checks id, digest, expiry and catalog and executes at most once.
        """
        with self._lock:
            return self._approve(request_id, approver, now or self._clock())

    def _approve(self, request_id, approver, now) -> CallOutcome:
        held = self._calls.get(request_id)
        base = self._base_record("continuation", request_id, held["tool_name"] if held else None, None)
        if held:
            base["message_id"] = held["message_id"]
            base["arguments_digest"] = held["arguments_digest"]
        pending = self.pipeline.pending_approval(self.trace_id, request_id)
        digest_seen = (self.authority.seen.get(request_id) or {}).get("request_digest")
        if held is None or held["outcome"] != APPROVAL_PENDING or pending is None or digest_seen is None:
            return self._refuse(base, "no_pending_approval", "no pending approval for this request")
        if request_id in self._continued:
            # One approver call per request, ever: the authority never sees a second approve request.
            return self._refuse(base, "no_pending_approval", "approval already continued for this request")
        self._continued.add(request_id)
        try:
            decision = approver(pending)
            if isinstance(decision, dict):
                decision = Decision.from_dict(decision)
            if not isinstance(decision, Decision):
                raise TypeError("approver returned no decision")
        except Exception:  # noqa: BLE001 - no decision, no continuation
            return self._refuse(base, "approver_failed", "approver returned no usable decision")
        try:
            result, observed = self.pipeline.continue_approval(
                self.trace_id, request_id, decision, digest_seen, now
            )
        except ContinuationRefused as err:
            return self._refuse(base, err.reason, "continuation refused by the pipeline")
        if result.status == "ok":
            text = jcs(result.data)
        else:
            text = _refusal_text(result.error.code, result.error.message, result.status)
        message = result.error.message if result.error else ""
        code = result.error.code if result.error else None
        outcome, certainty = classify(result.status, code, message, observed.execute_invoked)
        seen = self.authority.seen.get(request_id, {})
        rec = dict(
            base,
            capability_request_digest=digest_seen,
            decision=decision.decision,
            decision_digest=digest(decision.to_dict()),
            approval_id=decision.approval_id,
            stage=observed.stage,
            status=result.status,
            error_code=code,
            executed=bool(observed.execute_invoked),
            authority_execute_calls=seen.get("execute_calls", 0),
            result_digest=observed.result_digest,
            model_visible_digest=text_digest(text),
            outcome=outcome,
            effect_certainty=certainty,
            cancelled=False,
        )
        return self._finish(rec, text)

    def cancel_approval(self, request_id: str) -> CallOutcome:
        """Host-only: deny a held request for good. Nothing executes."""
        with self._lock:
            held = self._calls.get(request_id)
            base = self._base_record("continuation", request_id, held["tool_name"] if held else None, None)
            try:
                result, observed = self.pipeline.cancel_approval(self.trace_id, request_id)
            except ContinuationRefused as err:
                return self._refuse(base, err.reason, "cancel refused by the pipeline")
            text = _refusal_text(result.error.code, result.error.message, result.status)
            rec = dict(
                base,
                capability_request_digest=(self.authority.seen.get(request_id) or {}).get("request_digest"),
                decision=observed.decision, decision_digest=None, approval_id=None,
                stage=observed.stage, status=result.status, error_code=result.error.code,
                executed=False, authority_execute_calls=(self.authority.seen.get(request_id) or {}).get("execute_calls", 0),
                result_digest=observed.result_digest, model_visible_digest=text_digest(text),
                outcome=REJECTED, effect_certainty="none", cancelled=False,
            )
            return self._finish(rec, text)

    # -- records ----------------------------------------------------------------------
    def _base_record(self, kind, request_id, tool_name, motus_ids) -> dict:
        ids = {k: None for k in _MOTUS_ID_KEYS}
        for key, value in (motus_ids or {}).items():
            if key in ids and isinstance(value, str):
                ids[key] = value
        return {
            "kind": kind,
            "trace_id": self.trace_id,
            "request_id": request_id,
            "message_id": None,
            "tool_name": tool_name if isinstance(tool_name, str) else None,
            "motus": ids,
            "arguments_digest": None,
            "capability_request_digest": None,
            "decision": None,
            "decision_digest": None,
            "approval_id": None,
            "stage": None,
            "status": None,
            "error_code": None,
            "executed": False,
            "authority_execute_calls": 0,
            "result_digest": None,
            "model_visible_digest": None,
            "outcome": REJECTED,
            "effect_certainty": "none",
            "cancelled": False,
            "retried": False,
        }

    def _refuse(self, base: dict, code: str, message: str) -> CallOutcome:
        text = _refusal_text(code, message)
        rec = dict(
            base,
            status="rejected",
            error_code=code,
            executed=False,
            model_visible_digest=text_digest(text),
            unavailable="refused inside the adapter before or instead of the authority; no capability request exists",
        )
        return self._finish(rec, text)

    def _finish(self, rec: dict, text: str) -> CallOutcome:
        rec = dict(rec)
        rec["seq"] = len(self._records) + 1
        rec["retried"] = False
        self._records.append(rec)
        if rec["kind"] == "call" and rec["request_id"] is not None:
            self._calls.setdefault(rec["request_id"], rec)
        return CallOutcome(text=text, record=rec)

    # -- evidence ---------------------------------------------------------------------
    def close(self) -> None:
        with self._lock:
            if not self._closed:
                self._closed = True
                try:
                    self.pipeline.close_trace(self.trace_id)
                except Exception:  # noqa: BLE001 - closing is best effort, evidence is unaffected
                    pass

    def evidence(self) -> dict:
        with self._lock:
            header = {
                "adapter": "interplane-adapter-motus",
                "motus": {
                    "pin": MOTUS_PIN,
                    "installed_version": self._motus_version,
                    "run_id": self._motus_run_id,
                    "unavailable": UNAVAILABLE_MOTUS_ID,
                },
                "runtime_id": self.authority.runtime_id,
                "catalog_digest_at_start": self._catalog_at_start,
                "catalog_digest_at_export": self._catalog_digest(),
                "note": "tracing is observational; the INTERPLANE decision and receipt digests are the proof",
            }
            return build_bundle(self.trace_id, header, [dict(r) for r in self._records])
