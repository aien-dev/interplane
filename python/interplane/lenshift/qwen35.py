"""Lenshift dialect `qwen35` (version 1): Qwen3.5 chat-template tool calls as assistant text."""

import re
from typing import Any, Optional

from ..core import text_digest
from ._common import loads_strict, make_intent, new_turn, reject, serialize_result

NAME = "qwen35"

_THINK_RE = re.compile(r"\s*<think>(.*?)</think>", re.DOTALL)
_FUNCTION_RE = re.compile(r"<function=([^>]*)>(.*)</function>", re.DOTALL)
_OPEN, _CLOSE = "<tool_call>", "</tool_call>"
_TRUNCATED = "truncated tool call"


class _Malformed(Exception):
    def __init__(self, message: str, partial: bool = False):
        super().__init__(message)
        self.message = message
        self.partial = partial


def _strip_one_newline(value: str) -> str:
    if value.startswith("\n"):
        value = value[1:]
    if value.endswith("\n"):
        value = value[:-1]
    return value


def _json_guess(value: str) -> Any:
    try:
        parsed = loads_strict(value)
    except ValueError:
        return value
    return value if isinstance(parsed, str) else parsed


def _parse_function(body: str) -> tuple:
    """Parse ``<function=NAME>...</function>`` into (name, arguments)."""
    if "</function>" not in body:
        raise _Malformed(_TRUNCATED, partial=True)
    match = _FUNCTION_RE.fullmatch(body)
    if not match:
        raise _Malformed("tool call body is not a well-formed function block")
    name = match.group(1).strip()
    if not name:
        raise _Malformed("empty function name")
    rest = match.group(2)
    args: dict = {}
    pos = 0
    while True:
        start = rest.find("<parameter=", pos)
        if start < 0:
            if rest[pos:].strip():
                raise _Malformed("unexpected text inside function block")
            break
        if rest[pos:start].strip():
            raise _Malformed("unexpected text inside function block")
        gt = rest.find(">", start)
        if gt < 0:
            raise _Malformed(_TRUNCATED, partial=True)
        key = rest[start + len("<parameter=") : gt].strip()
        end = rest.find("</parameter>", gt)
        if end < 0:
            raise _Malformed(_TRUNCATED, partial=True)
        if not key:
            raise _Malformed("empty parameter name")
        if key in args:
            raise _Malformed(f"duplicate parameter: {key}")
        args[key] = _json_guess(_strip_one_newline(rest[gt + 1 : end]))
        pos = end + len("</parameter>")
    return name, args


def _parse_hermes(body: str) -> tuple:
    try:
        obj = loads_strict(body)
    except ValueError:
        raise _Malformed("tool call body is not valid JSON") from None
    if not isinstance(obj, dict) or not isinstance(obj.get("name"), str) or not obj["name"].strip():
        raise _Malformed("tool call JSON has no name")
    args = obj.get("arguments", {})
    if not isinstance(args, dict):
        raise _Malformed("arguments is not an object")
    return obj["name"].strip(), args


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
    pos = 0
    index = 0
    trailing = False
    parsed: list = []  # (index, hermes, name, args, source)
    while True:
        start = text.find(_OPEN, pos)
        if start < 0:
            pieces.append(text[pos:])
            break
        pieces.append(text[pos:start])
        end = text.find(_CLOSE, start + len(_OPEN))
        if end < 0:
            out.partial = True
            out.rejected.append(reject(index, _TRUNCATED, text_digest(text[start:])))
            break
        stop = end + len(_CLOSE)
        source = text[start:stop]
        body = text[start + len(_OPEN) : end].strip()
        src_digest = text_digest(source)
        try:
            if body.startswith("<function="):
                name, args = _parse_function(body)
                hermes = False
            elif body.startswith("{"):
                name, args = _parse_hermes(body)
                hermes = True
            elif not body:
                raise _Malformed("empty tool call")
            else:
                raise _Malformed("tool call body is neither a function block nor JSON")
        except _Malformed as err:
            out.partial = out.partial or err.partial
            out.rejected.append(reject(index, err.message, src_digest))
        else:
            parsed.append((index, hermes, name, args, src_digest))
        index += 1
        pos = stop
    out.text = "".join(pieces).strip()
    if parsed or out.rejected:
        last_close = text.rfind(_CLOSE)
        trailing = last_close >= 0 and bool(text[last_close + len(_CLOSE) :].strip())

    for idx, hermes, name, args, src_digest in parsed:
        intent = make_intent(
            dialect=NAME,
            dialect_version="hermes_json" if hermes else "1",
            model=model,
            trace_id=trace_id,
            turn=turn,
            index=idx,
            raw_name=name,
            arguments=args,
            source_call_id=None,
            source_digest=src_digest,
            coercion="none" if hermes else "json_guess",
            extensions={"trailing_text": True} if trailing else None,
        )
        if intent is None:
            out.rejected.append(reject(idx, "invalid tool name", src_digest))
            continue
        out.intents.append(intent)
        out.call_indexes.append(idx)
    out.rejected.sort(key=lambda r: r["index"])
    return out


def render_result(result: Any, intent: Optional[Any] = None) -> str:
    """The inner text of a ``role: tool`` message."""
    return f"<tool_response>\n{serialize_result(result)}\n</tool_response>"
