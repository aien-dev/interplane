"""INTERPLANE Core 0.1: protocol objects, canonical JSON, validation, lifecycle, limits.

Core executes nothing and authorizes nothing. Every object preserves unknown fields in ``extra``
and emits them back.
"""

import copy
import hashlib
import json
import re
from dataclasses import dataclass, field, fields, MISSING
from datetime import datetime
from decimal import Decimal
from enum import Enum
from typing import Any, ClassVar, Optional

PROTOCOL_VERSION = "0.1"
IMPLEMENTED_MAJOR = 0

ID_RE = re.compile(r"[A-Za-z0-9._:-]{1,128}")
TOOL_NAME_RE = re.compile(r"[A-Za-z0-9_.:-]{1,128}")
NAMESPACE_RE = re.compile(r"[a-z][a-z0-9_]*")
DIGEST_RE = re.compile(r"sha256:[0-9a-f]{64}")
VERSION_RE = re.compile(r"[0-9]+\.[0-9]+")
TIMESTAMP_RE = re.compile(r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})")

PARTY_KINDS = ("model", "runtime", "adapter", "operator", "other")
PAYLOAD_KINDS = (
    "tool_request",
    "capability_request",
    "decision",
    "result",
    "event",
    "catalog",
    "selection",
    "probe_report",
)
DECISIONS = ("authorized", "denied", "requires_approval", "not_found", "invalid")
RESULT_STATUSES = (
    "ok",
    "error",
    "timed_out",
    "denied",
    "requires_approval",
    "rejected",
    "not_found",
)
CONTENT_KINDS = (
    "runtime_instruction",
    "user_request",
    "model_generated",
    "tool_result",
    "workspace_content",
    "memory",
    "web_content",
    "document",
    "email",
    "skill",
    "external_provider",
    "unknown",
)
TRUST_LEVELS = (
    "trusted_runtime",
    "user_supplied",
    "workspace_untrusted",
    "external_untrusted",
    "unknown",
)
DEFAULT_CONTENT_KIND = "tool_result"
DEFAULT_TRUST = "unknown"
EVENT_NAMES = (
    "model_delta",
    "reasoning_delta",
    "tool_request",
    "tool_mapped",
    "tool_decision",
    "tool_execution_started",
    "tool_result",
    "tool_error",
    "continue",
    "complete",
    "cancelled",
)


class ErrorCode:
    """String constants for the closed ErrorCode set of common.schema.json."""

    UNSUPPORTED_VERSION = "unsupported_version"
    UNSUPPORTED_DIALECT = "unsupported_dialect"
    MALFORMED_ENVELOPE = "malformed_envelope"
    MALFORMED_TOOL_CALL = "malformed_tool_call"
    INVALID_ARGUMENTS = "invalid_arguments"
    OVERSIZED_ARGUMENTS = "oversized_arguments"
    DUPLICATE_REQUEST_ID = "duplicate_request_id"
    REPLAYED_MESSAGE = "replayed_message"
    UNKNOWN_CAPABILITY = "unknown_capability"
    CAPABILITY_NOT_FOUND = "capability_not_found"
    POLICY_DENIED = "policy_denied"
    APPROVAL_REQUIRED = "approval_required"
    EXECUTION_ERROR = "execution_error"
    EXECUTION_TIMEOUT = "execution_timeout"
    UNKNOWN_DECISION = "unknown_decision"
    RUNTIME_UNAVAILABLE = "runtime_unavailable"
    STALE_CAPABILITY = "stale_capability"

    ALL = frozenset(v for k, v in list(vars().items()) if k.isupper() and isinstance(v, str))


class ProtocolError(ValueError):
    """A structurally invalid protocol object. ``code`` is an ErrorCode string."""

    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


# ---------------------------------------------------------------------------
# Canonical JSON (RFC 8785) and digests
# ---------------------------------------------------------------------------


def _jcs_number(x: float) -> str:
    if x != x or x in (float("inf"), float("-inf")):
        raise ValueError("NaN and infinity are not representable in JSON")
    if x == 0:
        return "0"
    sign = "-" if x < 0 else ""
    tup = Decimal(repr(abs(x))).as_tuple()
    n = len(tup.digits) + tup.exponent  # decimal point position relative to digit string
    digits = "".join(str(d) for d in tup.digits).strip("0")
    k = len(digits)
    if k <= n <= 21:
        body = digits + "0" * (n - k)
    elif 0 < n <= 21:
        body = digits[:n] + "." + digits[n:]
    elif -6 < n <= 0:
        body = "0." + "0" * (-n) + digits
    else:
        e = n - 1
        exp = ("e+" if e >= 0 else "e-") + str(abs(e))
        body = digits[0] + ("." + digits[1:] if k > 1 else "") + exp
    return sign + body


def _jcs(value: Any, out: list) -> None:
    if value is None:
        out.append("null")
    elif value is True:
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, int):
        out.append(str(int(value)))
    elif isinstance(value, float):
        out.append(_jcs_number(value))
    elif isinstance(value, str):
        out.append(json.dumps(value, ensure_ascii=False))
    elif isinstance(value, (list, tuple)):
        out.append("[")
        for i, item in enumerate(value):
            if i:
                out.append(",")
            _jcs(item, out)
        out.append("]")
    elif isinstance(value, dict):
        out.append("{")
        if not all(isinstance(k, str) for k in value):
            raise TypeError("JSON object keys must be strings")
        keys = sorted(value, key=lambda k: k.encode("utf-16-be", "surrogatepass"))
        for i, key in enumerate(keys):
            if i:
                out.append(",")
            out.append(json.dumps(key, ensure_ascii=False))
            out.append(":")
            _jcs(value[key], out)
        out.append("}")
    else:
        raise TypeError(f"not JSON serializable: {type(value).__name__}")


def jcs(value: Any) -> str:
    """RFC 8785 canonical JSON text of ``value``."""
    out: list = []
    _jcs(value, out)
    return "".join(out)


def sha256_hex(data: "bytes | str") -> str:
    if isinstance(data, str):
        data = data.encode("utf-8")
    return hashlib.sha256(data).hexdigest()


def digest(value: Any) -> str:
    """``sha256:<hex>`` over the JCS bytes of ``value``."""
    return "sha256:" + sha256_hex(jcs(value))


def text_digest(text: str) -> str:
    """``sha256:<hex>`` over the UTF-8 bytes of ``text`` (no canonicalization)."""
    return "sha256:" + sha256_hex(text)


# ---------------------------------------------------------------------------
# Versioning
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProtocolVersion:
    major: int
    minor: int

    @classmethod
    def parse(cls, text: Any) -> "ProtocolVersion":
        if not isinstance(text, str) or not VERSION_RE.fullmatch(text):
            raise ProtocolError(
                ErrorCode.MALFORMED_ENVELOPE, "interplane_version must be MAJOR.MINOR"
            )
        major, minor = text.split(".")
        return cls(int(major), int(minor))

    def check(self) -> Optional[str]:
        """``unsupported_version`` unless the MAJOR is the one this implementation speaks."""
        return None if self.major == IMPLEMENTED_MAJOR else ErrorCode.UNSUPPORTED_VERSION

    def __str__(self) -> str:
        return f"{self.major}.{self.minor}"


def check_version(text: Any) -> Optional[str]:
    """ErrorCode or None for a version string."""
    try:
        return ProtocolVersion.parse(text).check()
    except ProtocolError as err:
        return err.code


# ---------------------------------------------------------------------------
# Model base: dataclass <-> dict with unknown-field preservation
# ---------------------------------------------------------------------------


class Model:
    """Base for schema objects. Subclasses are dataclasses whose last field is ``extra``."""

    KIND: ClassVar[Optional[str]] = None
    CODE: ClassVar[str] = ErrorCode.MALFORMED_ENVELOPE
    NESTED: ClassVar[dict] = {}  # field -> Model class, or [Model class] for lists
    TYPES: ClassVar[dict] = {}  # field -> tuple of accepted python types (non-null values)

    @classmethod
    def from_dict(cls, data: Any):
        if not isinstance(data, dict):
            raise ProtocolError(cls.CODE, f"{cls.__name__} must be a JSON object")
        if cls.KIND is not None and data.get("kind") != cls.KIND:
            raise ProtocolError(cls.CODE, f"kind must be {cls.KIND!r}")
        flds = {f.name: f for f in fields(cls) if f.name != "extra"}
        kwargs: dict = {}
        nulls: set = set()
        extra: dict = {}
        for key, value in data.items():
            if key == "kind" and cls.KIND is not None:
                continue
            if key not in flds:
                extra[key] = copy.deepcopy(value)
                continue
            if value is None:
                nulls.add(key)
                kwargs[key] = None
                continue
            want = cls.TYPES.get(key)
            if want and (
                not isinstance(value, want) or (bool not in want and isinstance(value, bool))
            ):
                raise ProtocolError(cls.CODE, f"{cls.__name__}.{key} has the wrong type")
            nested = cls.NESTED.get(key)
            if isinstance(nested, list):
                if not isinstance(value, list):
                    raise ProtocolError(cls.CODE, f"{cls.__name__}.{key} must be an array")
                value = [nested[0].from_dict(v) for v in value]
            elif nested is not None:
                value = nested.from_dict(value)
            else:
                value = copy.deepcopy(value)
            kwargs[key] = value
        for name, f in flds.items():
            required = f.default is MISSING and f.default_factory is MISSING
            if required and kwargs.get(name) is None:
                raise ProtocolError(cls.CODE, f"{cls.__name__}.{name} is required")
        obj = cls(**kwargs, extra=extra)
        obj.__dict__["_nulls"] = nulls
        return obj

    def keep_null(self, *names: str):
        """Mark fields that must be emitted as explicit nulls."""
        self.__dict__["_nulls"] = set(self.__dict__.get("_nulls", ())) | set(names)
        return self

    def to_dict(self) -> dict:
        out: dict = {}
        if self.KIND is not None:
            out["kind"] = self.KIND
        nulls = self.__dict__.get("_nulls", ())
        for f in fields(self):
            if f.name == "extra":
                continue
            value = getattr(self, f.name)
            if value is None:
                if f.name in nulls:
                    out[f.name] = None
                continue
            nested = self.NESTED.get(f.name)
            if isinstance(nested, list):
                out[f.name] = [v.to_dict() for v in value]
            elif nested is not None:
                out[f.name] = value.to_dict()
            else:
                out[f.name] = copy.deepcopy(value)
        for key, value in self.extra.items():
            out[key] = copy.deepcopy(value)
        return out


_DICT = (dict,)
_STR = (str,)


@dataclass
class Party(Model):
    kind: str
    id: str
    trust: Optional[str] = None
    content_kind: Optional[str] = None
    extra: dict = field(default_factory=dict)

    TYPES = {"kind": _STR, "id": _STR, "trust": _STR, "content_kind": _STR}


@dataclass
class ToolRef(Model):
    name: str
    namespace: Optional[str] = None
    extra: dict = field(default_factory=dict)
    CODE = ErrorCode.MALFORMED_TOOL_CALL
    TYPES = {"name": _STR, "namespace": _STR}

    @classmethod
    def from_dict(cls, data: Any):
        obj = super().from_dict(data)
        if not TOOL_NAME_RE.fullmatch(obj.name):
            raise ProtocolError(cls.CODE, "tool name does not match the ToolRef pattern")
        if obj.namespace is not None and not NAMESPACE_RE.fullmatch(obj.namespace):
            raise ProtocolError(cls.CODE, "tool namespace does not match the ToolRef pattern")
        return obj


@dataclass
class ToolRequest(Model):
    KIND = "tool_request"
    CODE = ErrorCode.MALFORMED_TOOL_CALL
    request_id: str
    tool: ToolRef
    arguments: dict
    provenance: dict
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"tool": ToolRef}
    TYPES = {"request_id": _STR, "arguments": _DICT, "provenance": _DICT, "extensions": _DICT}


@dataclass
class InputRecord(Model):
    """``input.schema.json``: one piece of material placed in front of the model (0.3 cut P2).

    ``trust`` is assigned by the runtime, never by the model. ``parent_id`` is always emitted, as
    an explicit ``null`` for host-registered inputs.
    """

    input_id: str
    content_kind: str
    trust: str
    source: Party
    origin: str
    content_digest: str
    trace_id: str
    parent_id: Optional[str] = None
    derived_from: list = field(default_factory=list)
    extra: dict = field(default_factory=dict)

    NESTED = {"source": Party}
    TYPES = {
        "input_id": _STR,
        "content_kind": _STR,
        "trust": _STR,
        "origin": _STR,
        "content_digest": _STR,
        "trace_id": _STR,
        "parent_id": _STR,
        "derived_from": (list,),
    }

    def to_dict(self) -> dict:
        out = super().to_dict()
        out.setdefault("parent_id", None)
        return out


@dataclass
class Exposure(Model):
    """``provenance.exposure`` on a tool_request (0.3 cut P2): the inputs visible to the model and
    the least trusted level among them. The pipeline computes it; a value arriving from a model or
    an envelope is never read (cut P3)."""

    inputs: list
    floor: str
    extra: dict = field(default_factory=dict)

    TYPES = {"inputs": (list,), "floor": _STR}


@dataclass
class CapabilityRequest(Model):
    KIND = "capability_request"
    request_id: str
    runtime: str
    capability: str
    arguments: dict
    tool: ToolRef
    mapping: dict
    runtime_effects: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"tool": ToolRef}
    TYPES = {
        "request_id": _STR,
        "runtime": _STR,
        "capability": _STR,
        "arguments": _DICT,
        "mapping": _DICT,
        "runtime_effects": _DICT,
        "extensions": _DICT,
    }


@dataclass
class Decision(Model):
    KIND = "decision"
    request_id: str
    decision: str
    authority: dict
    capability: Optional[str] = None
    reason: Optional[str] = None
    constraints: Optional[list] = None
    approval: Optional[dict] = None
    runtime_state: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    TYPES = {
        "request_id": _STR,
        "decision": _STR,
        "authority": _DICT,
        "capability": _STR,
        "reason": _STR,
        "constraints": (list,),
        "approval": _DICT,
        "runtime_state": _DICT,
        "extensions": _DICT,
    }

    @property
    def is_authorized(self) -> bool:
        """True only for the exact string ``authorized``; unknown values count as denied."""
        return self.decision == "authorized"

    @property
    def approval_id(self) -> Optional[str]:
        aid = (self.approval or {}).get("approval_id")
        return aid if isinstance(aid, str) and aid else None


@dataclass
class ErrorInfo(Model):
    code: str
    message: str
    retryable: Optional[bool] = None
    details: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    TYPES = {"code": _STR, "message": _STR, "retryable": (bool,), "details": _DICT}


@dataclass
class ToolResult(Model):
    KIND = "result"
    request_id: Optional[str]
    status: str
    data: Any = None
    error: Optional[ErrorInfo] = None
    decision: Optional[Decision] = None
    provenance: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"error": ErrorInfo, "decision": Decision}
    TYPES = {"status": _STR, "provenance": _DICT, "extensions": _DICT}


@dataclass
class Event(Model):
    KIND = "event"
    event: str
    seq: int
    request_id: Optional[str] = None
    turn: Optional[int] = None
    payload: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    TYPES = {"event": _STR, "seq": (int,), "turn": (int,), "payload": _DICT, "extensions": _DICT}


@dataclass
class CapabilityDescriptor(Model):
    name: str
    description: str
    parameters: dict
    canonical: Optional[ToolRef] = None
    domains: Optional[list] = None
    runtime_effects: Optional[dict] = None
    schema_digest: Optional[str] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"canonical": ToolRef}
    TYPES = {
        "name": _STR,
        "description": _STR,
        "parameters": _DICT,
        "domains": (list,),
        "runtime_effects": _DICT,
        "schema_digest": _STR,
    }


@dataclass
class Catalog(Model):
    KIND = "catalog"
    runtime: str
    catalog_version: str
    capabilities: list
    catalog_digest: Optional[str] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"capabilities": [CapabilityDescriptor]}
    TYPES = {"runtime": _STR, "catalog_version": _STR, "catalog_digest": _STR, "extensions": _DICT}

    def computed_digest(self) -> str:
        """Order-independent digest over capability names and schema digests."""
        pairs = sorted([c.name, c.schema_digest or digest(c.parameters)] for c in self.capabilities)
        return digest(pairs)

    def get(self, name: str) -> Optional[CapabilityDescriptor]:
        for cap in self.capabilities:
            if cap.name == name:
                return cap
        return None


@dataclass
class Selection(Model):
    KIND = "selection"
    runtime: str
    catalog_digest: str
    selector: dict
    requested_domains: list
    selected: list
    excluded: list
    always_include: Optional[list] = None
    measure: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    TYPES = {
        "runtime": _STR,
        "catalog_digest": _STR,
        "selector": _DICT,
        "requested_domains": (list,),
        "selected": (list,),
        "excluded": (list,),
        "always_include": (list,),
        "measure": _DICT,
        "extensions": _DICT,
    }


@dataclass
class ProbeReport(Model):
    KIND = "probe_report"
    probe_version: str
    endpoint: str
    model: str
    started_at: str
    probes: list
    profiles: list
    backend: Optional[str] = None
    backend_version: Optional[str] = None
    model_digest: Optional[str] = None
    finished_at: Optional[str] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    TYPES = {
        "probe_version": _STR,
        "endpoint": _STR,
        "model": _STR,
        "started_at": _STR,
        "probes": (list,),
        "profiles": (list,),
        "backend": _STR,
        "backend_version": _STR,
        "model_digest": _STR,
        "finished_at": _STR,
        "extensions": _DICT,
    }


@dataclass
class Envelope(Model):
    interplane_version: str
    message_id: str
    trace_id: str
    timestamp: str
    source: Party
    destination: Party
    payload: dict
    parent_id: Optional[str] = None
    digest: Optional[str] = None
    signature: Optional[dict] = None
    extensions: Optional[dict] = None
    extra: dict = field(default_factory=dict)

    NESTED = {"source": Party, "destination": Party}
    TYPES = {
        "interplane_version": _STR,
        "message_id": _STR,
        "trace_id": _STR,
        "timestamp": _STR,
        "payload": _DICT,
        "parent_id": _STR,
        "digest": _STR,
        "signature": _DICT,
        "extensions": _DICT,
    }

    def payload_object(self):
        """The payload as its typed object (kind must be one of the known kinds)."""
        classes = {
            c.KIND: c
            for c in (
                ToolRequest,
                CapabilityRequest,
                Decision,
                ToolResult,
                Event,
                Catalog,
                Selection,
                ProbeReport,
            )
        }
        cls = classes.get(self.payload.get("kind"))
        if cls is None:
            raise ProtocolError(ErrorCode.MALFORMED_ENVELOPE, "unknown payload kind")
        return cls.from_dict(self.payload)


@dataclass
class LenshiftTurn:
    """Output of a Lenshift dialect parser. Never carries authority."""

    dialect: str
    dialect_version: str
    parser_version: str
    model: Optional[str]
    text: str = ""
    reasoning_digest: Optional[str] = None
    intents: list = field(default_factory=list)
    rejected: list = field(default_factory=list)
    partial: bool = False
    # Position of each intent in the turn's emission order (not serialized).
    call_indexes: list = field(default_factory=list, compare=False, repr=False)

    def to_dict(self) -> dict:
        return {
            "dialect": self.dialect,
            "dialect_version": self.dialect_version,
            "parser_version": self.parser_version,
            "model": self.model,
            "text": self.text,
            "reasoning_digest": self.reasoning_digest,
            "intents": [i.to_dict() for i in self.intents],
            "rejected": [dict(r) for r in self.rejected],
            "partial": self.partial,
        }


# ---------------------------------------------------------------------------
# Envelope validation (structural only: never authorization)
# ---------------------------------------------------------------------------


def _is_id(value: Any) -> bool:
    return isinstance(value, str) and ID_RE.fullmatch(value) is not None


def _is_timestamp(value: Any) -> bool:
    if not isinstance(value, str) or not TIMESTAMP_RE.fullmatch(value):
        return False
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00").replace("z", "+00:00"))
    except ValueError:
        return False
    return True


def _valid_party(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and value.get("kind") in PARTY_KINDS
        and isinstance(value.get("id"), str)
        and 1 <= len(value["id"]) <= 256
    )


def _tool_request_problem(payload: Any) -> Optional[tuple]:
    """(code, field) for a structurally invalid ``tool_request`` payload, else None."""
    if not isinstance(payload, dict):
        return ErrorCode.MALFORMED_ENVELOPE, "payload"
    tool = payload.get("tool")
    if not isinstance(tool, dict) or not isinstance(tool.get("name"), str):
        return ErrorCode.MALFORMED_TOOL_CALL, "missing tool name"
    if not TOOL_NAME_RE.fullmatch(tool["name"]):
        return ErrorCode.MALFORMED_TOOL_CALL, "missing tool name"
    ns = tool.get("namespace")
    if ns is not None and not (isinstance(ns, str) and NAMESPACE_RE.fullmatch(ns)):
        return ErrorCode.MALFORMED_TOOL_CALL, "missing tool name"
    if not isinstance(payload.get("arguments"), dict):
        return ErrorCode.MALFORMED_TOOL_CALL, "arguments is not an object"
    if not _is_id(payload.get("request_id")):
        return ErrorCode.MALFORMED_ENVELOPE, "payload.request_id"
    prov = payload.get("provenance")
    if not isinstance(prov, dict) or not isinstance(prov.get("dialect"), str):
        return ErrorCode.MALFORMED_ENVELOPE, "payload.provenance"
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(prov.get("parser_version", ""))):
        return ErrorCode.MALFORMED_ENVELOPE, "payload.provenance.parser_version"
    if "extensions" in payload and not isinstance(payload["extensions"], dict):
        return ErrorCode.MALFORMED_ENVELOPE, "payload.extensions"
    return None


def validate_tool_request_payload(payload: Any) -> Optional[str]:
    """Structural check of a ``tool_request`` payload. Returns an ErrorCode or None."""
    problem = _tool_request_problem(payload)
    return problem[0] if problem else None


def envelope_problem(env: Any) -> Optional[tuple]:
    """``(ErrorCode, detail)`` for an invalid envelope, else None.

    ``detail`` is the offending field name (``malformed_envelope``), the unsupported MAJOR
    (``unsupported_version``) or the pinned dialect-style message (``malformed_tool_call``).

    Order: object shape, version (a MAJOR mismatch is rejected before the rest is read), required
    envelope fields, payload kind, then the ``tool_request`` structure. An unknown ``decision``
    string is not a structural error (receivers treat it as denied).
    """
    bad = ErrorCode.MALFORMED_ENVELOPE
    if not isinstance(env, dict):
        return bad, "envelope"
    version = env.get("interplane_version")
    try:
        parsed = ProtocolVersion.parse(version)
    except ProtocolError:
        return bad, "interplane_version"
    if parsed.check():
        return ErrorCode.UNSUPPORTED_VERSION, str(parsed.major)
    for key in ("message_id", "trace_id"):
        if not _is_id(env.get(key)):
            return bad, key
    if env.get("parent_id") is not None and not _is_id(env["parent_id"]):
        return bad, "parent_id"
    if not _is_timestamp(env.get("timestamp")):
        return bad, "timestamp"
    for key in ("source", "destination"):
        if not _valid_party(env.get(key)):
            return bad, key
    payload = env.get("payload")
    if not isinstance(payload, dict) or payload.get("kind") not in PAYLOAD_KINDS:
        return bad, "payload.kind"
    dig = env.get("digest")
    if dig is not None and not (isinstance(dig, str) and DIGEST_RE.fullmatch(dig)):
        return bad, "digest"
    for key in ("signature", "extensions"):
        if env.get(key) is not None and not isinstance(env[key], dict):
            return bad, key
    if payload["kind"] == "tool_request":
        return _tool_request_problem(payload)
    if payload["kind"] == "decision":
        if not _is_id(payload.get("request_id")):
            return bad, "payload.request_id"
        if not isinstance(payload.get("decision"), str):
            return bad, "payload.decision"
    return None


def validate_envelope(env: Any) -> Optional[str]:
    """Structural validation of an envelope dict. Returns an ErrorCode or None."""
    problem = envelope_problem(env)
    return problem[0] if problem else None


# ---------------------------------------------------------------------------
# Limits and replay ledger
# ---------------------------------------------------------------------------


@dataclass
class Limits:
    max_argument_bytes: int = 65536
    max_requests_per_turn: int = 32
    max_text_length: int = 4096

    @classmethod
    def from_dict(cls, data: Optional[dict]) -> "Limits":
        known = {f.name for f in fields(cls)}
        return cls(**{k: v for k, v in (data or {}).items() if k in known and isinstance(v, int)})

    def arguments_oversized(self, arguments: Any) -> bool:
        return len(jcs(arguments).encode("utf-8")) > self.max_argument_bytes

    def truncate(self, text: str) -> str:
        return text[: self.max_text_length]


class RequestLedger:
    """Remembers message_ids (replay) and request_ids (duplicates) per trace."""

    def __init__(self) -> None:
        self._messages: dict = {}
        self._requests: dict = {}

    def admit(self, trace_id: str, message_id: str, request_id: Optional[str]) -> Optional[str]:
        """Record the ids; returns ``replayed_message``/``duplicate_request_id`` or None."""
        messages = self._messages.setdefault(trace_id, set())
        if message_id in messages:
            return ErrorCode.REPLAYED_MESSAGE
        messages.add(message_id)
        if request_id is not None:
            requests = self._requests.setdefault(trace_id, set())
            if request_id in requests:
                return ErrorCode.DUPLICATE_REQUEST_ID
            requests.add(request_id)
        return None

    def admit_message(self, trace_id: str, message_id: str) -> Optional[str]:
        """``replayed_message`` when this message_id was already seen in the trace."""
        messages = self._messages.setdefault(trace_id, set())
        if message_id in messages:
            return ErrorCode.REPLAYED_MESSAGE
        messages.add(message_id)
        return None

    def admit_request_id(self, trace_id: str, request_id: str) -> Optional[str]:
        requests = self._requests.setdefault(trace_id, set())
        if request_id in requests:
            return ErrorCode.DUPLICATE_REQUEST_ID
        requests.add(request_id)
        return None


# ---------------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------------


class State(str, Enum):
    PROPOSED = "PROPOSED"
    REJECTED = "REJECTED"
    MAPPED = "MAPPED"
    DENIED = "DENIED"
    REQUIRES_APPROVAL = "REQUIRES_APPROVAL"
    AUTHORIZED = "AUTHORIZED"
    EXECUTING = "EXECUTING"
    SUCCEEDED = "SUCCEEDED"
    FAILED = "FAILED"
    TIMED_OUT = "TIMED_OUT"


TERMINAL = frozenset({State.REJECTED, State.DENIED, State.SUCCEEDED, State.FAILED, State.TIMED_OUT})


class LifecycleError(RuntimeError):
    """An illegal transition was attempted."""


class Lifecycle:
    """The Crossveil per-request state machine.

    The only way into AUTHORIZED is ``apply_decision`` with a ``Decision`` whose ``decision`` is
    exactly ``authorized`` and whose ``request_id`` matches. There is no setter for ``state`` and no
    method that builds a Decision.
    """

    __slots__ = ("_request_id", "_state", "_decision", "_approval_id")

    def __init__(self, request_id: Optional[str]) -> None:
        self._request_id = request_id
        self._state = State.PROPOSED
        self._decision: Optional[Decision] = None
        self._approval_id: Optional[str] = None

    @property
    def state(self) -> State:
        return self._state

    @property
    def request_id(self) -> Optional[str]:
        return self._request_id

    @property
    def decision(self) -> Optional[Decision]:
        return self._decision

    def _require(self, *allowed: State) -> None:
        if self._state not in allowed:
            raise LifecycleError(f"illegal transition from {self._state.value}")

    def reject(self) -> State:
        """Core/CrossAxis refusal (PROPOSED) or a runtime invalid/not_found (via decision)."""
        self._require(State.PROPOSED, State.MAPPED)
        self._state = State.REJECTED
        return self._state

    def map(self) -> State:
        self._require(State.PROPOSED)
        self._state = State.MAPPED
        return self._state

    def apply_decision(self, decision: Any) -> State:
        """Feed a runtime decision. Unknown decision strings are DENIED."""
        if not isinstance(decision, Decision):
            raise LifecycleError("a runtime Decision object is required")
        if self._request_id is not None and decision.request_id != self._request_id:
            raise LifecycleError("decision cites a different request_id")
        if self._state is State.MAPPED:
            self._decision = decision
            value = decision.decision
            if value == "authorized":
                self._state = State.AUTHORIZED
            elif value in ("invalid", "not_found"):
                self._state = State.REJECTED
            elif value == "requires_approval":
                self._state = State.REQUIRES_APPROVAL
                self._approval_id = decision.approval_id
            else:  # denied and every unknown value
                self._state = State.DENIED
            return self._state
        if self._state is State.REQUIRES_APPROVAL:
            cited = decision.approval_id
            if cited is None or cited != self._approval_id or decision is self._decision:
                raise LifecycleError("approval requires a new decision citing the approval_id")
            self._decision = decision
            self._state = State.AUTHORIZED if decision.decision == "authorized" else State.DENIED
            return self._state
        raise LifecycleError(f"cannot decide in state {self._state.value}")

    def cancel(self) -> State:
        """REQUIRES_APPROVAL -> DENIED without a decision: a host cancel, or a continuation the
        pipeline refused to honour (fail closed). It can only deny."""
        self._require(State.REQUIRES_APPROVAL)
        self._state = State.DENIED
        return self._state

    def start_execution(self) -> State:
        self._require(State.AUTHORIZED)
        if self._decision is None or not self._decision.is_authorized:
            raise LifecycleError("no authorizing decision")
        self._state = State.EXECUTING
        return self._state

    def finish(self, status: str) -> State:
        """EXECUTING -> SUCCEEDED | FAILED | TIMED_OUT from a result status."""
        self._require(State.EXECUTING)
        self._state = {"ok": State.SUCCEEDED, "timed_out": State.TIMED_OUT}.get(
            status, State.FAILED
        )
        return self._state
