"""Lenshift dialect `openai_stream` (version 1): a streamed OpenAI-compatible response body (SSE text).

The fragments are assembled into one assistant message, which is parsed with the ``openai`` rules.
A stream that did not finish cleanly is rejected call by call ("truncated tool call"), never guessed.
"""

from typing import Any, Optional

from ..core import jcs, text_digest
from . import openai
from ._common import loads_strict, new_turn, reject

NAME = "openai_stream"
COMPLETE = ("stop", "tool_calls", "function_call")


class StreamError(ValueError):
    pass


def events(body: str) -> list:
    """The data of each complete SSE event, in order."""
    out, data = [], []
    for line in body.split("\n"):
        if line.endswith("\r"):
            line = line[:-1]
        if line == "":
            if data:
                out.append("\n".join(data))
            data = []
        elif line == "data":
            data.append("")
        elif line.startswith("data:"):
            value = line[5:]
            data.append(value[1:] if value.startswith(" ") else value)
    return out


def _u64(value: Any) -> Optional[int]:
    """``value`` when it is a JSON integer in 0..2^64-1 (what a strict reader holds as u64), else None."""
    if type(value) is int and 0 <= value <= 0xFFFFFFFFFFFFFFFF:
        return value
    return None


def _keep_first(slot: dict, key: str, value: Any, what: str) -> None:
    if not isinstance(value, str) or not value:
        return
    if slot.get(key) is None:
        slot[key] = value
    elif slot[key] != value:
        raise StreamError(f"conflicting tool call {what}")


def assemble(body: str):
    """``(message, finish_reason)`` for a stream body; raises ``StreamError``."""
    text, reasoning, calls = [], [], {}
    finish: Optional[str] = None
    done = False
    for data in events(body):
        if done:
            raise StreamError("event after [DONE]")
        if data == "[DONE]":
            done = True
            continue
        try:
            chunk = loads_strict(data)
        except ValueError:
            chunk = None
        if not isinstance(chunk, dict):
            raise StreamError("stream chunk is not a JSON object")
        choices = chunk.get("choices")
        if not isinstance(choices, list):
            continue
        for choice in choices:
            if not isinstance(choice, dict):
                raise StreamError("stream choice is not an object")
            if "index" in choice and _u64(choice["index"]) != 0:
                raise StreamError("multiple choices are not supported")
            delta = choice.get("delta")
            if isinstance(delta, dict):
                if isinstance(delta.get("content"), str):
                    text.append(delta["content"])
                for key in ("reasoning_content", "reasoning"):
                    if isinstance(delta.get(key), str):
                        reasoning.append(delta[key])
                fragments = delta.get("tool_calls")
                for frag in fragments if isinstance(fragments, list) else []:
                    index = _u64(frag.get("index")) if isinstance(frag, dict) else None
                    if index is None:
                        raise StreamError("tool call fragment has no index")
                    slot = calls.setdefault(index, {})
                    _keep_first(slot, "id", frag.get("id"), "id")
                    function = frag.get("function")
                    if isinstance(function, dict):
                        _keep_first(slot, "name", function.get("name"), "name")
                        if isinstance(function.get("arguments"), str):
                            slot["arguments"] = slot.get("arguments", "") + function["arguments"]
            reason = choice.get("finish_reason")
            if reason is not None and not isinstance(reason, str):
                raise StreamError("finish_reason is not a string")
            if reason is not None:
                if finish is None:
                    finish = reason
                elif finish != reason:
                    raise StreamError("conflicting finish_reason")
    message = {"role": "assistant", "content": "".join(text)}
    if reasoning:
        message["reasoning_content"] = "".join(reasoning)
    if calls:
        out = []
        for index in sorted(calls):
            slot = calls[index]
            function = {k: slot[k] for k in ("name", "arguments") if slot.get(k) is not None}
            call = {"type": "function", "function": function}
            if slot.get("id") is not None:
                call["id"] = slot["id"]
            out.append(call)
        message["tool_calls"] = out
    return message, finish


def _retag(turn) -> None:
    turn.dialect = NAME
    for intent in turn.intents:
        intent.provenance["dialect"] = NAME


def parse(input: Any, model: str, trace_id: str, turn: int):
    try:
        if not isinstance(input, str):
            raise StreamError("stream is not text")
        message, finish = assemble(input)
    except StreamError as e:
        out = new_turn(NAME, model)
        body = input if isinstance(input, str) else jcs(input)
        out.rejected.append(reject(0, str(e), text_digest(body)))
        return out
    out = openai.parse(message, model, trace_id, turn)
    _retag(out)
    if finish in COMPLETE:
        return out
    cut = [
        (index, intent.provenance["source_digest"], intent.provenance.get("source_call_id"))
        for index, intent in zip(out.call_indexes, out.intents)
    ] + [(r["index"], r["source_digest"], r.call_id) for r in out.rejected]
    out.intents, out.call_indexes = [], []
    out.rejected = [reject(i, "truncated tool call", d, c) for i, d, c in sorted(cut, key=lambda x: x[0])]
    out.partial = bool(cut)
    return out


render_result = openai.render_result
