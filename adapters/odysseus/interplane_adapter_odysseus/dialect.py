"""Lenshift dialect ``odysseus_text``: the text-mode tool-call forms Odysseus's own parser accepts.

Registered adapter-locally (``make_registry()``), which is the proof that Core needs no change to
gain a dialect. Shapes are copied from Odysseus 2992bf6:
  * ``src/tool_parsing.py:1288-1310`` (``parse_tool_blocks`` docstring and pattern order:
    fenced ``` blocks, then ``[TOOL_CALL]``, then XML ``<tool_call>/<invoke>``) and the regexes at
    ``:33-37`` (fence), ``:91-92`` (TOOL_CALL), ``:106-136`` (XML).
  * ``tests/test_redos_xml_tool_parsers.py:49`` (``<tool_call><invoke name="bash">
    <parameter name="command">ls -la</parameter></invoke></tool_call>``), ``:99``
    (``[TOOL_CALL]{tool => "shell", args => {--command "ls"}}[/TOOL_CALL]``) and ``:111-114``
    (a fence whose body is ``<invoke>`` markup).

Hermes JSON ``<tool_call>{"name":..,"arguments":{..}}</tool_call>`` is NOT parsed here: the core
``qwen35`` dialect already covers it (``hermes_json``). Such a body is rejected with a pointer.

Differences from Odysseus, all deliberate: scanning is forward-only (no backtracking regexes);
names are never renamed or lowercased (Lenshift never renames, so ``shell`` stays ``shell`` and the
mapping table answers ``unknown_capability``); parameter values stay strings (CrossAxis coerces by
schema); an unterminated call is rejected as truncated, never guessed.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Optional

from interplane.core import text_digest
from interplane.lenshift import Registry
from interplane.lenshift import openai as _openai
from interplane.lenshift import qwen35 as _qwen35
from interplane.lenshift._common import make_intent, new_turn, reject

from .catalog import tool_names

NAME = "odysseus_text"
_TRUNCATED = "truncated tool call"
_CODE_TAGS = {"bash": "command", "python": "code"}
# fenced tag -> the argument its raw (non-JSON) body fills, as tool_execution._MCP_ARG_PARSERS
# (:632 read_file) and the code tools do. Tags outside this table need a JSON object body.
_PRIMARY = {
    "bash": "command", "python": "code", "read_file": "path", "ls": "path", "glob": "pattern",
    "grep": "pattern", "web_search": "query", "web_fetch": "url",
}
_FIRST_LINE_ONLY = frozenset({"read_file", "ls", "glob", "grep", "web_search", "web_fetch"})
_XML_WRAPPERS = (("<tool_call>", "</tool_call>"), ("<function_call>", "</function_call>"))

_NAMES: Optional[frozenset] = None


def _known() -> frozenset:
    global _NAMES
    if _NAMES is None:
        _NAMES = frozenset(tool_names())
    return _NAMES


class _Bad(Exception):
    def __init__(self, message: str, partial: bool = False):
        super().__init__(message)
        self.message, self.partial = message, partial


# -- XML <invoke> ---------------------------------------------------------------------------------


def _attr_name(tag_head: str) -> str:
    """The ``name="..."`` / ``name='...'`` attribute of an opening tag head (text before ``>``)."""
    pos = tag_head.find("name=")
    if pos < 0 or pos + 5 >= len(tag_head) or tag_head[pos + 5] not in "\"'":
        raise _Bad("missing name attribute")
    quote = tag_head[pos + 5]
    end = tag_head.find(quote, pos + 6)
    if end < 0:
        raise _Bad("missing name attribute")
    value = tag_head[pos + 6 : end].strip()
    if not value:
        raise _Bad("empty name attribute")
    return value


def _invokes(body: str) -> list:
    """All ``<invoke name=..>`` blocks in ``body`` as ``(name, args, source)``; forward-only."""
    out, pos = [], 0
    while True:
        start = body.find("<invoke", pos)
        if start < 0:
            return out
        gt = body.find(">", start)
        if gt < 0:
            raise _Bad(_TRUNCATED, partial=True)
        name = _attr_name(body[start + len("<invoke") : gt])
        end = body.find("</invoke>", gt)
        if end < 0:
            raise _Bad(_TRUNCATED, partial=True)
        out.append((name, _parameters(body[gt + 1 : end]), body[start : end + len("</invoke>")]))
        pos = end + len("</invoke>")


def _parameters(inner: str) -> dict:
    args: dict = {}
    pos = 0
    while True:
        start = inner.find("<parameter", pos)
        if start < 0:
            return args
        gt = inner.find(">", start)
        if gt < 0:
            raise _Bad(_TRUNCATED, partial=True)
        key = _attr_name(inner[start + len("<parameter") : gt])
        end = inner.find("</parameter>", gt)
        if end < 0:
            raise _Bad(_TRUNCATED, partial=True)
        if key in args:
            raise _Bad(f"duplicate parameter: {key}")
        args[key] = inner[gt + 1 : end].strip()  # tool_parsing._parse_xml_invoke: pval.strip()
        pos = end + len("</parameter>")


# -- the three forms ------------------------------------------------------------------------------


def _scan_fenced(text: str) -> tuple:
    """Pattern 1: ```<tool> fences. Returns (calls, rejects, spans); each call is
    ``(start, name, args, source)``."""
    calls: list = []
    spans: list = []
    pos = 0
    known = _known()
    while True:
        start = text.find("```", pos)
        if start < 0:
            break
        tag_start = start + 3
        tag_end = tag_start
        while tag_end < len(text) and (text[tag_end].isalnum() or text[tag_end] == "_"):
            tag_end += 1
        tag = text[tag_start:tag_end].lower()
        nxt = text[tag_end : tag_end + 1]
        if not tag or nxt == "-" or (tag not in known and tag not in ("python", "xml", "json")):
            if tag:  # a non-tool fence (```text, ```rust ...): skip it whole
                skip = text.find("```", tag_end)
                pos = skip + 3 if skip >= 0 else len(text)
            else:
                pos = tag_start
            continue
        eol = text.find("\n", tag_end)
        close = text.find("```", tag_end)
        if close < 0:
            if tag in known:
                calls.append((start, None, _Bad(_TRUNCATED, partial=True), text[start:]))
                spans.append((start, len(text)))
            break
        if eol < 0 or eol > close:
            inline, body = text[tag_end:close].strip(), ""
        else:
            inline, body = text[tag_end:eol].strip(), text[eol + 1 : close]
        stop = close + 3
        source = text[start:stop]
        pos = stop
        body_s = body.strip()
        if "<invoke" in body_s:  # a fence wrapping <invoke> markup (test_redos:111-114)
            try:
                for name, args, src in _invokes(body_s):
                    calls.append((start, name, args, src))
                spans.append((start, stop))
            except _Bad as err:
                calls.append((start, None, err, source))
                spans.append((start, stop))
            continue
        if tag not in known:
            continue  # ```json / ```xml with no invoke markup: ordinary prose
        if not (inline or body_s):
            continue  # an empty fence runs nothing (tool_parsing.py:1325-1335)
        try:
            args = _fence_args(tag, inline, body)
        except _Bad as err:
            calls.append((start, None, err, source))
            spans.append((start, stop))
            continue
        if args is None:
            continue  # display text, e.g. ```bash {title="x"}
        calls.append((start, tag, args, source))
        spans.append((start, stop))
    return calls, spans


def _fence_args(tag: str, inline: str, body: str) -> Optional[dict]:
    if inline:
        if tag in _CODE_TAGS or inline[0] not in "{[":
            return None  # fence metadata, not arguments (tool_parsing._fenced_tool_call)
        content = f"{inline}\n{body}".strip() if body.strip() else inline
        try:
            obj = json.loads(content)
        except ValueError:
            return None
        if not isinstance(obj, dict):
            raise _Bad("arguments is not an object")
        return obj
    stripped = body.strip()
    if tag not in _CODE_TAGS and stripped.startswith("{"):
        try:
            obj = json.loads(stripped)
        except ValueError:
            raise _Bad("arguments is not valid JSON") from None
        if not isinstance(obj, dict):
            raise _Bad("arguments is not an object")
        return obj
    key = _PRIMARY.get(tag)
    if key is None:
        raise _Bad("fenced body is not a JSON object")
    value = stripped.split("\n", 1)[0].strip() if tag in _FIRST_LINE_ONLY else stripped
    return {key: value}


def _scan_tool_call_blocks(text: str) -> tuple:
    """Pattern 2: ``[TOOL_CALL]{tool => "x", args => {--key "v"}}[/TOOL_CALL]``."""
    low = text.lower()
    calls: list = []
    spans: list = []
    pos = 0
    while True:
        start = low.find("[tool_call]", pos)
        if start < 0:
            break
        end = low.find("[/tool_call]", start + 11)
        if end < 0:
            calls.append((start, None, _Bad(_TRUNCATED, partial=True), text[start:]))
            spans.append((start, len(text)))
            break
        stop = end + len("[/tool_call]")
        source = text[start:stop]
        inner = text[start + 11 : end].strip()
        pos = stop
        spans.append((start, stop))
        try:
            name, args = _parse_tool_call_inner(inner)
        except _Bad as err:
            calls.append((start, None, err, source))
        else:
            calls.append((start, name, args, source))
    return calls, spans


def _parse_tool_call_inner(inner: str) -> tuple:
    if not (inner.startswith("{") and inner.endswith("}")):
        raise _Bad("tool call block is not a braced object")
    inner = inner[1:-1]
    head = _find_key(inner, "tool")
    if head is None:
        raise _Bad("missing tool name")
    value, _ = head
    name = value.strip().strip("\"'")
    if not name or not (name.replace("_", "").isalnum()):
        raise _Bad("missing tool name")
    arg_pos = _find_arrow_key(inner, "args")
    if arg_pos is None:
        return name, {}
    brace = inner.find("{", arg_pos)
    close = inner.rfind("}")  # through the LAST brace, as tool_parsing.py:849-852
    if brace < 0 or close < brace:
        raise _Bad("args is not a braced block")
    body = inner[brace + 1 : close].strip()
    if not body:
        return name, {}
    try:
        obj = json.loads("{" + body + "}")
    except ValueError:
        obj = None
    if isinstance(obj, dict):
        return name, obj
    return name, _flag_args(body)


def _find_arrow_key(text: str, key: str) -> Optional[int]:
    """Index just after ``key =>`` / ``key:`` / ``key=`` (key at a word boundary); else None."""
    pos = 0
    while True:
        i = text.find(key, pos)
        if i < 0:
            return None
        before_ok = i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")
        j = i + len(key)
        while j < len(text) and text[j] in " \t":
            j += 1
        for arrow in ("=>", ":", "="):
            if text.startswith(arrow, j):
                if before_ok:
                    return j + len(arrow)
                break
        pos = i + len(key)


def _find_key(text: str, key: str) -> Optional[tuple]:
    after = _find_arrow_key(text, key)
    if after is None:
        return None
    rest = text[after:].lstrip()
    if rest[:1] in "\"'":
        q = rest[0]
        end = rest.find(q, 1)
        return (rest[1:end], after) if end > 0 else None
    end = 0
    while end < len(rest) and (rest[end].isalnum() or rest[end] == "_"):
        end += 1
    return (rest[:end], after) if end else None


def _flag_args(body: str) -> dict:
    """``--key "value" --other 'v' --bare word`` -> dict (the ``--command "ls"`` shape)."""
    args: dict = {}
    i, n = 0, len(body)
    while i < n:
        while i < n and body[i] in " \t\r\n,":
            i += 1
        if i >= n:
            break
        if not body.startswith("--", i):
            raise _Bad("unrecognized args form")
        i += 2
        k = i
        while i < n and (body[i].isalnum() or body[i] in "_-"):
            i += 1
        key = body[k:i]
        if not key:
            raise _Bad("unrecognized args form")
        while i < n and body[i] in " \t":
            i += 1
        if i < n and body[i] in "\"'":
            q = body[i]
            end = body.find(q, i + 1)
            if end < 0:
                raise _Bad(_TRUNCATED, partial=True)
            value, i = body[i + 1 : end], end + 1
        else:
            v = i
            while i < n and body[i] not in " \t\r\n":
                i += 1
            value = body[v:i]
        if key in args:
            raise _Bad(f"duplicate parameter: {key}")
        args[key] = value
    return args


def _scan_xml(text: str) -> tuple:
    """Pattern 3: ``<tool_call><invoke name=..><parameter name=..>v</parameter></invoke></tool_call>``."""
    calls: list = []
    spans: list = []
    pos = 0
    while True:
        hit = None
        for opener, closer in _XML_WRAPPERS:
            i = text.find(opener, pos)
            if i >= 0 and (hit is None or i < hit[0]):
                hit = (i, opener, closer)
        if hit is None:
            break
        start, opener, closer = hit
        end = text.find(closer, start + len(opener))
        if end < 0:
            calls.append((start, None, _Bad(_TRUNCATED, partial=True), text[start:]))
            spans.append((start, len(text)))
            break
        stop = end + len(closer)
        source = text[start:stop]
        body = text[start + len(opener) : end].strip()
        pos = stop
        spans.append((start, stop))
        if body.startswith("{") or body.startswith("<function="):
            calls.append((start, None, _Bad("tool call body is JSON or <function=> form: use the qwen35 dialect"), source))
            continue
        try:
            found = _invokes(body)
            if not found:
                raise _Bad("tool call body has no invoke block")
        except _Bad as err:
            calls.append((start, None, err, source))
            continue
        for name, args, src in found:
            calls.append((start, name, args, source if len(found) == 1 else src))
    return calls, spans


# -- Lenshift entry points --------------------------------------------------------------------------


def parse(input: Any, model: str, trace_id: str, turn: int):
    out = new_turn(NAME, model)
    if not isinstance(input, str):
        out.rejected.append(reject(0, "assistant turn is not text", None))
        return out
    text = input
    think = text.lstrip()
    if think.startswith("<think>") and "</think>" in think:
        end = think.find("</think>")
        reasoning = think[len("<think>") : end].strip()
        out.reasoning_digest = text_digest(reasoning) if reasoning else None
        text = think[end + len("</think>") :]

    # Odysseus's precedence (tool_parsing.py:1317-1346): fenced, else [TOOL_CALL], else XML.
    calls, spans = _scan_fenced(text)
    if not calls:
        calls, spans = _scan_tool_call_blocks(text)
    if not calls:
        calls, spans = _scan_xml(text)

    kept, pos = [], 0
    for a, b in sorted(spans):
        kept.append(text[pos:a])
        pos = max(pos, b)
    kept.append(text[pos:])
    out.text = "".join(kept).strip()

    for index, (_, name, args, source) in enumerate(calls):
        digest = text_digest(source)
        if name is None:  # a _Bad
            err = args
            out.partial = out.partial or err.partial
            out.rejected.append(reject(index, err.message, digest))
            continue
        intent = make_intent(
            dialect=NAME, dialect_version="1", model=model, trace_id=trace_id, turn=turn,
            index=index, raw_name=name, arguments=args, source_call_id=None,
            source_digest=digest, coercion="none",
        )
        if intent is None:
            out.rejected.append(reject(index, "invalid tool name", digest))
            continue
        out.intents.append(intent)
        out.call_indexes.append(index)
    return out


_DESC_KEYS = ("path", "command", "code", "pattern", "query", "url")


def _describe(intent: Optional[Any]) -> str:
    if intent is None:
        return "tool"
    name = (intent.provenance or {}).get("raw_name") or intent.tool.name
    for key in _DESC_KEYS:
        value = intent.arguments.get(key)
        if isinstance(value, str) and value.strip():
            return f"{name}: {value.strip().splitlines()[0][:80]}"
    return str(name)


def render_result(result: Any, intent: Optional[Any] = None) -> str:
    """One tool result as Odysseus's text-mode ``format_tool_result`` text (tool_execution.py:1359).

    Reimplements only the ``output`` and ``error`` branches (:1367-1371 and :1409-1410) plus the
    ``### <description>`` header and the ``**data:**`` fallback; a differential test pins it to the
    real function when Odysseus is importable. Odysseus wraps a whole round of these once in
    ``untrusted_context_message`` (agent_loop.py:3095-3120): use :func:`round_message` for that.
    """
    data = result.to_dict() if hasattr(result, "to_dict") else result
    desc = _describe(intent)
    status = data.get("status")
    if status == "ok":
        payload = data.get("data")
        parts = [f"### {desc}"]
        if isinstance(payload, dict) and "output" in payload:
            parts.append(f"```\n{payload['output']}\n```")
            if payload.get("exit_code") not in (0, None):
                parts.append(f"**exit_code:** {payload['exit_code']}")
        else:
            parts.append(f"**data:**\n```json\n{json.dumps(payload, indent=2, default=str, ensure_ascii=False)}\n```")
        return "\n".join(parts)
    err = data.get("error") or {}
    if status in ("denied", "requires_approval", "rejected", "not_found"):
        desc = f"{desc.split(':', 1)[0]}: BLOCKED"  # tool_execution.py:1067 shape
    return f"### {desc}\n**Error:** {err.get('message')}"


def round_message(rendered: list, odysseus: Any = None, arm_gate: bool = True) -> dict:
    """Odysseus's own wrapper for a round of text-mode results (needs a real Odysseus)."""
    from . import _odysseus

    ody = odysseus if odysseus is not None else _odysseus.load()
    if ody is None:
        raise RuntimeError("odysseus runtime not available")
    return ody.prompt_security.untrusted_context_message(
        "tool execution results", "\n\n".join(rendered), arm_tool_gate=arm_gate
    )


def make_registry() -> Registry:
    """A registry with openai, qwen35 (which already covers Hermes JSON) and ``odysseus_text``.

    Local on purpose: the global Lenshift registry is left untouched.
    """
    registry = Registry()
    registry.register("openai", _openai)
    registry.register("qwen35", _qwen35)
    registry.register(NAME, sys.modules[__name__])
    return registry
