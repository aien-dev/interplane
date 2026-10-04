"""Lenshift dialect `openai` (version 1): an OpenAI-compatible assistant message."""

from typing import Any

from ..core import jcs, text_digest
from ._common import loads_strict, make_intent, new_turn, reject, serialize_result

NAME = "openai"


def _content_text(content: Any) -> str:
    if content is None:
        return ""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(
            p["text"] for p in content if isinstance(p, dict) and isinstance(p.get("text"), str)
        )
    return ""


def _reasoning(message: dict):
    for key in ("reasoning_content", "reasoning"):
        value = message.get(key)
        if isinstance(value, str) and value:
            return text_digest(value)
    return None


def parse(input: Any, model: str, trace_id: str, turn: int):
    out = new_turn(NAME, model)
    if not isinstance(input, dict):
        out.rejected.append(reject(0, "assistant message is not an object", None))
        return out
    out.text = _content_text(input.get("content"))
    out.reasoning_digest = _reasoning(input)

    calls = input.get("tool_calls")
    if not isinstance(calls, list) or not calls:
        legacy = input.get("function_call")
        calls = [{"function": legacy, "_legacy": True}] if isinstance(legacy, dict) else []

    for index, call in enumerate(calls):
        source = call["function"] if isinstance(call, dict) and call.get("_legacy") else call
        try:
            src_digest = text_digest(jcs(source))
        except (TypeError, ValueError):
            src_digest = None
        cid = call.get("id") if isinstance(call, dict) and not call.get("_legacy") else None
        if not isinstance(call, dict):
            out.rejected.append(reject(index, "tool call is not an object", src_digest, cid))
            continue
        legacy = call.get("_legacy", False)
        function = call["function"] if legacy else call.get("function")
        if not isinstance(function, dict):
            out.rejected.append(reject(index, "tool call has no function object", src_digest, cid))
            continue
        name = function.get("name")
        if not isinstance(name, str) or not name:
            out.rejected.append(reject(index, "missing tool name", src_digest, cid))
            continue
        raw_args = function.get("arguments")
        try:
            if not isinstance(raw_args, str):
                raise ValueError
            args = loads_strict(raw_args)
        except ValueError:
            out.rejected.append(reject(index, "arguments is not valid JSON", src_digest, cid))
            continue
        if not isinstance(args, dict):
            out.rejected.append(reject(index, "arguments is not an object", src_digest, cid))
            continue
        call_id = None if legacy else call.get("id")
        if not isinstance(call_id, str):
            call_id = None
        intent = make_intent(
            dialect=NAME,
            dialect_version="1",
            model=model,
            trace_id=trace_id,
            turn=turn,
            index=index,
            raw_name=name,
            arguments=args,
            source_call_id=call_id,
            source_digest=src_digest,
            coercion="none",
        )
        if intent is None:
            out.rejected.append(reject(index, "invalid tool name", src_digest, cid))
            continue
        out.intents.append(intent)
        out.call_indexes.append(index)
    return out


def render_result(result: Any, intent: Any = None) -> dict:
    """A ``role: tool`` message for ``result``."""
    request_id = result.request_id if hasattr(result, "request_id") else result.get("request_id")
    call_id = None
    if intent is not None:
        call_id = intent.provenance.get("source_call_id")
    return {
        "role": "tool",
        "tool_call_id": call_id or request_id,
        "content": serialize_result(result),
    }
