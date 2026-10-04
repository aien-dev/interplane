"""Helpers shared by dialect modules."""

import json
from typing import Any, Optional

from ..core import (
    ID_RE,
    TOOL_NAME_RE,
    NAMESPACE_RE,
    LenshiftTurn,
    ToolRef,
    ToolRequest,
    jcs,
)

PARSER_VERSION = "1.0.0"


def split_name(raw_name: str) -> Optional[ToolRef]:
    """Canonicalize a tool name: split on the last '.' only when the prefix is a namespace."""
    namespace = None
    name = raw_name
    if "." in raw_name:
        prefix, _, rest = raw_name.rpartition(".")
        if NAMESPACE_RE.fullmatch(prefix) and rest:
            namespace, name = prefix, rest
    if not TOOL_NAME_RE.fullmatch(name):
        return None
    return ToolRef(name=name, namespace=namespace).keep_null("namespace")


def request_id_for(source_call_id: Optional[str], trace_id: str, turn: int, index: int) -> str:
    if isinstance(source_call_id, str) and ID_RE.fullmatch(source_call_id):
        return source_call_id
    return f"{trace_id}:t{turn}:c{index}"


def make_intent(
    *,
    dialect: str,
    dialect_version: str,
    model: str,
    trace_id: str,
    turn: int,
    index: int,
    raw_name: str,
    arguments: dict,
    source_call_id: Optional[str],
    source_digest: str,
    coercion: str,
    extensions: Optional[dict] = None,
) -> Optional[ToolRequest]:
    tool = split_name(raw_name)
    if tool is None:
        return None
    provenance = {
        "dialect": dialect,
        "dialect_version": dialect_version,
        "model": model,
        "parser_version": PARSER_VERSION,
        "source_turn": turn,
        "source_call_id": source_call_id,
        "source_digest": source_digest,
        "raw_name": raw_name,
        "coercion": coercion,
    }
    req = ToolRequest(
        request_id=request_id_for(source_call_id, trace_id, turn, index),
        tool=tool,
        arguments=arguments,
        provenance=provenance,
        extensions=extensions,
    )
    return req


class Rejected(dict):
    """A ``rejected`` entry; ``call_id`` (not serialized) is the dialect-native call id, if any."""

    call_id: Optional[str] = None


def reject(
    index: int, message: str, source_digest: Optional[str], call_id: Optional[str] = None
) -> Rejected:
    entry = Rejected(
        index=index, code="malformed_tool_call", message=message, source_digest=source_digest
    )
    entry.call_id = call_id if isinstance(call_id, str) else None
    return entry


def serialize_result(result: Any) -> str:
    """Canonical serialization of a result for a tool-role message (never raw secrets)."""
    data = result.to_dict() if hasattr(result, "to_dict") else result
    if data.get("status") == "ok":
        return jcs(data.get("data"))
    err = data.get("error") or {}
    return jcs(
        {
            "error": {"code": err.get("code"), "message": err.get("message")},
            "status": data.get("status"),
        }
    )


def new_turn(dialect: str, model: str) -> LenshiftTurn:
    return LenshiftTurn(
        dialect=dialect, dialect_version="1", parser_version=PARSER_VERSION, model=model
    )


def _no_constant(name: str):
    raise ValueError(f"non-finite number {name}")


def loads_strict(text: str) -> Any:
    """json.loads that refuses NaN/Infinity (not representable in canonical JSON)."""
    return json.loads(text, parse_constant=_no_constant)
