"""Lenshift dialect `aien_legacy` (version 1): aien-cli's textual ``<tool_call>`` JSON protocol.

Ground truth: aien-sovereign-core 7580039, crates/aien-cli/src/client.rs:350-419 (parser) and
main.rs:330-340 (result reinjection). See spec/LENSHIFT.md.
"""

import re
from typing import Any, Optional

from ..core import text_digest
from ._common import loads_strict, make_intent, new_turn, reject, serialize_result

NAME = "aien_legacy"

_WS = " \t\n\r\x0b\x0c"  # ASCII whitespace, identical in both reference implementations
_OPEN, _CLOSE = "<tool_call>", "</tool_call>"
_THINK_RE = re.compile(r"[ \t\n\r\x0b\x0c]*<think>(.*?)</think>", re.DOTALL)
_FENCE_RE = re.compile(
    r'```(?:json|tool_call)?[ \t\n\r\x0b\x0c]*(\{[ \t\n\r\x0b\x0c]*"name"[ \t\n\r\x0b\x0c]*:'
    r'[ \t\n\r\x0b\x0c]*"[^"]+".*?\})[ \t\n\r\x0b\x0c]*```',
    re.DOTALL,
)
_TRUNCATED = "truncated tool call"


class _Malformed(Exception):
    pass


_BAD = object()


def _load(text: str) -> Any:
    try:
        return loads_strict(text)
    except ValueError:
        return _BAD


def _repair_json(raw: str) -> tuple:
    """aien-cli ``parse_tool_call_json`` (client.rs:350-379). Returns (value, repairs)."""
    body = raw.strip(_WS)
    value = _load(body)
    if value is not _BAD:
        return value, []
    if body.endswith("]"):  # client.rs:355-362
        value = _load(body[:-1] + "}")
        if value is not _BAD:
            return value, ["bracket_to_brace"]
    opens, closes = body.count("{"), body.count("}")  # client.rs:363-377
    if opens > closes:
        candidate = body
        if body.count('"') % 2 == 1:
            candidate += '"'
        candidate += "}" * (opens - closes)
        value = _load(candidate)
        if value is not _BAD:
            return value, ["close_braces"]
    raise _Malformed("tool call body is not valid JSON")


def _decode(raw: str) -> tuple:
    """-> (name, arguments, repairs) or raise _Malformed."""
    obj, repairs = _repair_json(raw)
    name = obj.get("name") if isinstance(obj, dict) else None
    if not isinstance(name, str) or not name:
        raise _Malformed("tool call JSON has no name")
    if name == "tool_name":  # the prompt's placeholder, dropped silently by aien-cli
        raise _Malformed("placeholder tool name")
    args = obj.get("arguments", {})
    if not isinstance(args, dict):
        raise _Malformed("arguments is not an object")
    return name, args, repairs


def parse(input: Any, model: str, trace_id: str, turn: int):
    out = new_turn(NAME, model)
    if not isinstance(input, str):
        out.rejected.append(reject(0, "assistant turn is not text", None))
        return out
    text = input
    think = _THINK_RE.match(text)
    if think:
        reasoning = think.group(1).strip()
        out.reasoning_digest = text_digest(reasoning) if reasoning else None
        text = text[think.end() :]

    pieces: list = []
    blocks: list = []  # (raw_body, source_text, form)
    if _OPEN in text:
        pos = 0
        while True:
            start = text.find(_OPEN, pos)
            if start < 0:
                pieces.append(text[pos:])
                break
            pieces.append(text[pos:start])
            end = text.find(_CLOSE, start + len(_OPEN))
            if end < 0:
                out.partial = True
                blocks.append((None, text[start:], "tool_call"))
                break
            stop = end + len(_CLOSE)
            blocks.append((text[start + len(_OPEN) : end], text[start:stop], "tool_call"))
            pos = stop
    else:
        pos = 0
        for m in _FENCE_RE.finditer(text):
            pieces.append(text[pos : m.start()])
            blocks.append((m.group(1), m.group(0), "fenced_json"))
            pos = m.end()
        pieces.append(text[pos:])
    out.text = "".join(pieces).strip()

    for index, (body, source, form) in enumerate(blocks):
        digest = text_digest(source)
        if body is None:
            out.rejected.append(reject(index, _TRUNCATED, digest))
            continue
        try:
            name, args, repairs = _decode(body)
        except _Malformed as err:
            out.rejected.append(reject(index, str(err), digest))
            continue
        intent = make_intent(
            dialect=NAME,
            dialect_version="1",
            model=model,
            trace_id=trace_id,
            turn=turn,
            index=index,
            raw_name=name,
            arguments=args,
            source_call_id=None,
            source_digest=digest,
            coercion="none",
        )
        if intent is None:
            out.rejected.append(reject(index, "invalid tool name", digest))
            continue
        if repairs:
            intent.provenance["repairs"] = repairs
        if form != "tool_call":
            intent.provenance["form"] = form
        out.intents.append(intent)
        out.call_indexes.append(index)
    return out


def render_result(result: Any, intent: Optional[Any] = None) -> dict:
    """A user-role message: ``<tool_response name="NAME">\\n<json>\\n</tool_response>``."""
    name = "unknown"
    if intent is not None and intent.provenance.get("raw_name"):
        name = intent.provenance["raw_name"]
    return {
        "role": "user",
        "content": f'<tool_response name="{name}">\n{serialize_result(result)}\n</tool_response>',
    }
