# Lenshift: model-native representation <-> canonical intent

Lenshift owns representation differences. It never executes, never authorizes, never decides
whether a capability exists. It produces `LenshiftTurn`:

```
LenshiftTurn {
  dialect, dialect_version, parser_version, model,
  text: string              // the model's prose, with tool-call markup and think blocks removed
  reasoning_digest: Digest|null   // sha256 of the reasoning channel; raw reasoning not kept
  intents: [tool_request]   // structurally valid calls, in emission order
  rejected: [{ index, code, message, source_digest }]   // malformed calls, in emission order
  partial: bool             // a tool call was cut off (streaming/truncation)
}
```

`rejected` entries also become `rejected` results (`malformed_tool_call`) so the model learns the
call was not understood. Partial calls are rejected, never guessed.

Every intent's provenance records `dialect`, `parser_version`, `model`, `source_turn`,
`source_call_id` (if the dialect has ids), `source_digest` (sha256 of the exact dialect-native
representation of that one call), `raw_name`, `coercion`.

Tool name canonicalization: `raw_name` is split on the **last** `.` into `namespace` + `name` only
when the prefix matches `^[a-z][a-z0-9_]*$`; otherwise `namespace = null` and `name = raw_name`.
Lenshift never renames tools.

Request ids: `source_call_id` when the dialect provides one, else `"<trace_id>:t<turn>:c<index>"`
assigned by the caller. Lenshift itself does not know the trace; it emits `request_id = null` and
the pipeline fills it. (In the reference implementations the parse function takes the trace and
turn so ids are filled in one step.)

## Dialect `openai` (version 1)

Input: one assistant message object `{role, content, tool_calls?, reasoning_content?}` as returned
by an OpenAI-compatible `/v1/chat/completions` (non-streamed, or streamed-and-assembled by the
caller).

- `tool_calls[i].function.name` -> raw_name; `tool_calls[i].id` -> source_call_id.
- `function.arguments` is a JSON string: parse; must be an object. Not JSON -> `malformed_tool_call`
  "arguments is not valid JSON". JSON but not an object -> `malformed_tool_call` "arguments is not an object".
- Missing or empty name -> `malformed_tool_call`.
- `content` -> text (null -> ""). `reasoning_content` / `reasoning` -> reasoning_digest.
- Legacy `function_call` (single) is accepted as one call with no id.
- `coercion = "none"`.

Render result -> `{"role":"tool","tool_call_id":<source_call_id or request_id>,"content":<string>}`
where content is `data` JSON-serialized (canonical) for `ok`, otherwise
`{"error": {"code":..., "message":...}, "status": ...}` serialized. Never raw secrets.

## Dialect `qwen35` (version 1)

Grounded in the official `Qwen/Qwen3.5-9B` chat template (sha256 a4aee8af…, lines 53, 105-137).

Input: the assistant turn as text (what the server returned in `content`, possibly with the
reasoning included inline).

- Reasoning: a leading `<think>...</think>` block (possibly empty) is removed; its content is digested.
- Tool calls: each `<tool_call>` ... `</tool_call>` block whose body is `<function=NAME>` ... `</function>`
  with zero or more `<parameter=KEY>` ... `</parameter>` children.
  - KEY and NAME: trimmed; empty -> `malformed_tool_call`.
  - Value: the text between the tags with exactly one leading and one trailing newline stripped
    (the template emits `\n` + value + `\n`). Internal newlines are preserved.
  - Typing: `coercion = "json_guess"`: if the value parses as JSON and the result is not a string,
    use the parsed value; otherwise keep the string. (The template serializes non-string arguments
    with `tojson`; the schema-driven coercion that serving engines do is CrossAxis's job.)
  - Duplicate KEY within one call -> `malformed_tool_call`.
- `<tool_call>` without a closing tag, `<function=` without `</function>`, or `<parameter=`
  without `</parameter>` -> `partial = true` and a `rejected` entry "truncated tool call".
- A `<tool_call>` body that is bare JSON `{"name":..,"arguments":{..}}` (the Qwen3 Hermes form that
  older templates and some servers still emit) is accepted with `dialect_version = "hermes_json"`
  recorded in provenance, so qualification can tell which form the model actually produced.
- Text after the last `</tool_call>` is kept in `text` (the template forbids it, so provenance
  notes `trailing_text = true` in extensions; it is not an error).
- `text` is everything outside think and tool_call blocks, trimmed.

Render result -> the content of a `role: tool` message: `<tool_response>\n<serialized>\n</tool_response>`
with the same serialization as the openai dialect. (The chat template wraps tool messages itself;
Lenshift emits the inner text and the caller places it in a `tool` role message.)

Both forms of a Qwen3.5 tool call are real inputs: raw XML when the caller reads the model text,
and OpenAI `tool_calls` when a serving engine's parser converted it. The `openai` dialect handles
the second; qualification runs both.

## Dialect `ajax` — reserved, not implemented

Ajax (announced 2026-10-02 as a Qwen3.5-9B fine-tune for Odysseus) had no published weights,
model card, license, tokenizer or chat template on 2026-10-04. No parser exists. Adding one must
require only a new module under `dialects/ajax` and `lenshift/ajax.*`, plus fixtures captured
from the official artifact: no change to Core. The reference implementations' dialect registry is
the abstraction test for that.

## Fixtures

`dialects/fixtures/<dialect>/<name>.json`: `{"input": ..., "model": ..., "expected": LenshiftTurn-without-digests, "expected_digests": {...}}`.
Required qwen35 fixtures: valid single call; multiple calls; malformed (unclosed function);
reasoning plus call; plain answer with no tool; unknown tool name (parses fine); partial/truncated
call; multi-line parameter value; JSON-typed parameter; hermes_json form. Required openai fixtures:
single; multiple; arguments not JSON; arguments not object; missing name; reasoning_content present;
plain answer; legacy function_call.

Fixture provenance: inputs are constructed from the official template grammar, not captured from a
live model, until the qualification run under `qualification/` captures real outputs and pins them.
