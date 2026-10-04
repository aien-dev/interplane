# Interplane Probe: what does this endpoint really support?

`interplane probe <base_url> [--model M]` runs a fixed list of real requests against an
OpenAI-compatible endpoint and writes a `probe_report`. Every verdict cites a request and response
digest. Nothing is inferred from the model name.

Probes (version 0.1.0), each a real request:

| Probe | PASS when |
|---|---|
| `chat.basic` | a one-line prompt returns non-empty content |
| `chat.streaming` | `stream: true` returns SSE chunks that assemble to non-empty content |
| `tools.native` | with one `tools` entry and a prompt that needs it, the response has `tool_calls` with the right name and object arguments |
| `tools.text_qwen35` | same prompt without `tools` but with the Qwen3.5 template-style instruction yields a parseable `<tool_call><function=...>` block |
| `tools.parallel` | a prompt needing two independent calls yields two `tool_calls` in one turn |
| `tools.result_replay` | after injecting a `role: tool` result the model produces a final answer that uses it |
| `tools.unknown_refusal` | asked for a tool that is not in `tools`, the model does not call a nonexistent tool (DEGRADED if it does but names one of the provided tools) |
| `reasoning.channel` | `reasoning_content` or `reasoning` is present, or `<think>` appears in content |
| `reasoning.disable` | with the engine's disable mechanism (`chat_template_kwargs.enable_thinking=false`, `think=false`, `reasoning_effort=none`, tried in order) the reasoning channel is absent |
| `json.structured` | `response_format: {"type":"json_object"}` returns valid JSON |
| `context.8k` / `context.16k` / `context.32k` | a prompt of that size returns a correct needle answer (DEGRADED when it answers but wrong; FAIL on error) |
| `models.list` | `/v1/models` lists the model |

Profiles:

| Profile | Required probes |
|---|---|
| `interplane.core.0.1` | `chat.basic`, `models.list` |
| `lenshift.openai.1` | `tools.native`, `tools.result_replay` |
| `lenshift.qwen35.1` | `tools.text_qwen35`, `tools.result_replay` |
| `relayline.tool_replay.1` | `tools.result_replay`, `chat.streaming` |

Probe is a qualification tool and a post-install smoke test. It does not make a serving engine
part of INTERPLANE: it speaks plain HTTP to whatever is there. Credentials come from
`INTERPLANE_PROBE_API_KEY` and are never written to the report.
