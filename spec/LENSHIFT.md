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
by an OpenAI-compatible `/v1/chat/completions` (non-streamed, or streamed and assembled by the
caller; dialect `openai_stream` below assembles a stream).

- `tool_calls[i].function.name` -> raw_name; `tool_calls[i].id` -> source_call_id.
- `function.arguments` is a JSON string: parse; must be an object. Not JSON, absent, or not a string
  (an already-decoded object, as Ollama's own `/api/chat` sends, is not the OpenAI-compatible form)
  -> `malformed_tool_call` "arguments is not valid JSON". JSON but not an object ->
  `malformed_tool_call` "arguments is not an object".
- Missing or empty name -> `malformed_tool_call`.
- `content` -> text (null -> ""). `reasoning_content` / `reasoning` -> reasoning_digest.
- Legacy `function_call` (single) is accepted as one call with no id.
- `coercion = "none"`.

Render result -> `{"role":"tool","tool_call_id":<source_call_id or request_id>,"content":<string>}`
where content is `data` JSON-serialized (canonical) for `ok`, otherwise
`{"error": {"code":..., "message":...}, "status": ...}` serialized. Never raw secrets.

## Dialect `openai_stream` (version 1)

Input: the body of a streamed OpenAI-compatible `/v1/chat/completions` response (`stream: true`), as
one string of Server-Sent Events, exactly as the server sent it. Lenshift assembles the fragments
into one assistant message and parses it with the `openai` rules above. Grounded in the six streamed
captures under `qualification/captures/qwen35-{ollama,llamacpp,sglang}/h{1,2}-*.json`: Ollama sends
each tool call whole; llama.cpp sends `id`, `name` and `"{"` first and the arguments in later pieces;
SGLang does the same but repeats `"id": null, "name": null` in every later piece and sends
`content: "\n\n"` before the call.

Framing (the `data` subset of the WHATWG Server-Sent Events format):
- Split on `\n`; a trailing `\r` on a line is removed. A line `data` or `data:<value>` appends
  `<value>` (one leading space removed) to the event's data lines. Lines starting with `:` and all
  other fields (`event`, `id`, `retry`) are ignored. A blank line ends an event; its data is the data
  lines joined with `\n`. An event not ended by a blank line before the end of input is discarded.
- Event data `[DONE]` ends the stream; any later event is a stream error. Any other data must be a
  JSON object (strict JSON, as for arguments); otherwise a stream error.

Assembly, chunk by chunk in arrival order:
- `choices` absent, not an array, or empty (for example a usage chunk): ignored. A choice that is
  not an object: stream error "stream choice is not an object". A choice with an `index` that is not
  the integer `0`: stream error "multiple choices are not supported" (an absent `index` counts as 0).
- `delta.content` and `delta.reasoning_content` / `delta.reasoning` strings are concatenated; `null`
  is skipped. `delta.role` is ignored.
- `delta.tool_calls[]` (ignored when not an array): each fragment must be an object with an integer
  `index` from 0 to 2^64-1 (`1.0` is not an integer), else stream error "tool call fragment has no
  index". Per index: the first non-empty string `id` and the first non-empty string
  `function.name` are kept; a later, different non-empty value is a stream error ("conflicting tool
  call id" / "conflicting tool call name"); `null` and `""` are skipped. `function.arguments`
  strings are concatenated.
- `finish_reason`: must be a string or `null`, else stream error "finish_reason is not a string"; the
  first non-null value is kept; a later, different non-null value is a stream
  error "conflicting finish_reason".

Assembled message: `{"role": "assistant", "content": <text>, "tool_calls": [...]}` with calls in
ascending `index`, each `{"type": "function", "function": {"name", "arguments"}}` plus `"id"` when one
arrived; `"reasoning_content"` is present when reasoning text arrived. A call with no name or no
arguments keeps the field absent and is rejected by the `openai` rules.

Completeness: the stream is complete only when `finish_reason` is `stop`, `tool_calls` or
`function_call`. Otherwise (absent, `length`, `content_filter`, anything else) every call of the turn
is rejected `malformed_tool_call` "truncated tool call" with its own index, digest and id, and
`partial = true` when there was at least one call. Text is kept. A cut-off stream is never guessed,
even when a call's arguments already parse.

Stream errors and non-string input: no intents, `text = ""`, one `rejected` entry at index 0
(`malformed_tool_call`, the error message, `source_digest` = sha256 of the body text, or of its JCS
form for a non-string), `partial = false`.

Intents carry `dialect = "openai_stream"`, `dialect_version = "1"`; `source_digest` is of the
assembled call object (JCS), as for `openai`. Render result is the same as `openai`.

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

## Dialect `aien_legacy` (version 1)

The textual protocol of `aien-cli` (aien-sovereign-core 7580039). Ground truth, all under
`crates/aien-cli/src/`: parser `client.rs:350-419` (`parse_tool_call_json`, `extract_tool_calls`),
prompt-side format `client.rs:100-160`, result reinjection `main.rs:330-340` (the same shape
repeats at `main.rs:414-421` and `:537-553`). Input: the assistant turn as text.

aien-cli streams reasoning in a separate `reasoning` / `reasoning_content` delta and keeps it out of
the text it parses (`client.rs:297-325`); it never looks for `<think>` in the text.

- Tool calls: each `<tool_call>` ... `</tool_call>` block, body trimmed, whose body is JSON
  `{"name": string, "arguments": object}` (Hermes style). aien-cli's regex is
  `(?s)<tool_call>\s*(.*?)\s*(?:</tool_call>|$)` (`client.rs:383`). Blocks are parsed in order and
  every one is dispatched (`main.rs:330-340`); there is no per-turn cap.
- Repairs aien-cli applies, tried in this order only after a strict parse fails (`client.rs:350-379`):
  1. `bracket_to_brace`: a body ending in `]` has it replaced by `}` (`:355-362`).
  2. `close_braces`: when `{` outnumbers `}` (counted over every character, strings included), a `"`
     is appended if the body has an odd number of `"`, then the missing `}` are appended
     (`:363-377`).
  3. Otherwise the call is dropped without a trace.
  There is **no** trailing-comma repair and **no** single-quote repair; those inputs are rejected.
  A repaired intent carries `provenance.repairs` (a list of the names above); `coercion = "none"`.
- Missing `arguments` means `{}` (`client.rs:392`). aien-cli forwards any other value unchecked.
- Name `tool_name` (the placeholder in aien-cli's system prompt) is dropped silently (`:391`).
- Fenced fallback: only when the `<tool_call>` pass produced no call, aien-cli scans for
  `` ```(?:json|tool_call)?\s*(\{\s*"name"\s*:\s*"[^"]+".*?\})\s*``` `` (`:399-416`) and parses each
  match with the same repairs.
- Result rendering (`main.rs:338`): a **user-role** message
  `{"role":"user","content":"<tool_response name=\"NAME\">\n<json>\n</tool_response>"}`. The
  attribute is `name` (the tool name as the model wrote it, unescaped); there is no call id.

Lenshift's rules (differences from aien-cli are marked **deviation**):

- Reasoning: a leading `<think>...</think>` block is removed and digested, as in `qwen35`.
  Lenshift addition: aien-cli never sees it in text.
- Blocks are parsed in emission order; `index` counts every block, rejected ones included.
- Whitespace in trimming, the fence pattern and the think prefix is ASCII (` \t\n\r\v\f`) in both
  implementations, so they stay byte-identical. aien-cli's `\s`/`trim` also match other Unicode
  whitespace.
- **deviation** Unterminated `<tool_call>` (no `</tool_call>`): `partial = true`, `rejected`
  "truncated tool call", never guessed. aien-cli accepts it to end of text (`client.rs:383`, test
  at `:448-454`). Repair 2 therefore applies only to a block the model closed itself.
- **deviation** Rejected with `malformed_tool_call` instead of dropped silently: body not JSON after
  repairs ("tool call body is not valid JSON"), JSON that is not an object with a non-empty string
  `name` ("tool call JSON has no name"), name `tool_name` ("placeholder tool name"), `arguments`
  present but not an object ("arguments is not an object"), illegal tool name ("invalid tool name").
- **deviation** The fenced fallback runs only when the text contains no `<tool_call>` marker at all,
  so a malformed or truncated tagged call is never replaced by an unrelated fenced block. Fenced
  intents carry `provenance.form = "fenced_json"`. Fenced blocks are removed from `text`.
- `source_digest` is of the exact block (`<tool_call>` through `</tool_call>`, or the whole fence).
  `source_call_id` is null. `dialect_version = "1"`.
- `text` is everything outside think, tool_call and matched fence blocks, trimmed.

Render result -> `{"role":"user","content":"<tool_response name=\"NAME\">\n<serialized>\n</tool_response>"}`
with the shared serialization of the openai dialect. NAME is the intent's `raw_name`; without an
intent (for example a rejected call) it is `unknown`. aien-cli serializes its own result object with
`serde_json::to_string`; Lenshift does not reproduce that shape, only the envelope.

Versus `qwen35`'s `hermes_json` variant: the JSON body grammar is the same, but aien_legacy adds the
two repairs, the placeholder-name rule, the fenced fallback and the user-role rendering, and
`qwen35` accepts the XML `<function=...>` form which aien-cli does not. They are separate dialects
with separate parsers.

## Dialect `ajax` — reserved, not implemented

Ajax (announced 2026-10-02 as a Qwen3.5-9B fine-tune for Odysseus) had no published weights,
model card, license, tokenizer or chat template on 2026-10-04. No parser exists. Adding one must
require only a new module under `dialects/ajax` and `lenshift/ajax.*`, plus fixtures captured
from the official artifact: no change to Core. The reference implementations' dialect registry is
the abstraction test for that.

## Fixtures

`dialects/fixtures/<dialect>/<name>.json`: `{"input": ..., "model": ..., "expected": LenshiftTurn-without-digests, "expected_digests": {...}}`.
Required openai_stream fixtures: every streamed capture under `qualification/captures` byte for byte
(with its `sha256_response`); fragmented call; repeated null fragments; two interleaved calls; cut
mid-arguments; cut after complete arguments; `finish_reason` length; unterminated last event; CRLF
and comment framing; streamed plain text, complete and cut; assembled arguments not JSON; each stream
error. Fixtures record `expected_digests`, and both runners compare every recorded digest.
Required qwen35 fixtures: valid single call; multiple calls; malformed (unclosed function);
reasoning plus call; plain answer with no tool; unknown tool name (parses fine); partial/truncated
call; multi-line parameter value; JSON-typed parameter; hermes_json form. Required aien_legacy fixtures: valid single call; multiple calls; reasoning plus call; repaired bracket; repaired missing braces; trailing comma (rejected); single quotes (rejected); unterminated (rejected, partial); fenced json fallback; placeholder name (rejected); missing arguments; arguments not object; plain answer. Required openai fixtures:
single; multiple; arguments not JSON; arguments not object; missing name; reasoning_content present;
plain answer; legacy function_call.

Fixture provenance: inputs are constructed from the official template grammar, not captured from a
live model, until the qualification run under `qualification/` captures real outputs and pins them.
