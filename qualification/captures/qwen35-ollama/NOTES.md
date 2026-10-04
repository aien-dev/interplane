# Qwen3.5 9B on Ollama: captured outputs (2026-10-04)

Facts only. Each `NN-name.json` holds `request` (exact body), `endpoint`, `response` (exact raw body as a string; for streams the concatenated SSE text), `sha256_response`. All requests: temperature 0, seed 42. Tool set: read_file, list_dir, write_file, send_email.

## Environment
- Host: NVIDIA DGX Spark GB10, Linux aarch64. Ollama 0.34.0, started with `setsid nohup ollama serve` (log ~/.ollama-serve.log), listening 127.0.0.1:11434.
- Tag: `qwen3.5:9b` (official library tag; the default 9b tag, Q4_K_M). Architecture qwen35, 9.0B params, quantization Q4_K_M, context length 262144, capabilities: completion, vision, tools, thinking. License Apache-2.0.
- Manifest digest (sha256 of manifest file): 56671c2ab9385f9cfcb404638e32cd62d88e3501d44822208363c010179a3c90 (`ollama list` ID 56671c2ab938). Model blob sha256-02d45dc1cf451ba2475ac33b301c2dd8f985abe4c182ce04a1f2f5bf0260278d (5.6 GB), projector blob sha256-f836f08f9211... (0.9 GB).
- `ollama ps` during runs: 15 GB, 100% GPU, context 262144.
- Default sampling parameters in the model: temperature 1, top_k 20, top_p 0.95, presence_penalty 1.5 (captures override temperature and seed only; top_k/top_p/presence_penalty were the model defaults).

## Template vs official Jinja
- `ollama show --template` returns exactly `{{ .Prompt }}` (ollama_template.txt, 13 bytes). Modelfile has `RENDERER qwen3.5` and `PARSER qwen3.5` (ollama_modelfile.txt). So the official Jinja is NOT used; prompt rendering and tool-call parsing happen in Ollama's built-in Go renderer/parser named qwen3.5. Its source was not inspected; there is no template text to diff line by line.
- Behavioral evidence about the renderer (not a source diff): on the native path, prompt_tokens for the one-call request was 488. The hand-built raw prompt following the official Jinja (system block with `<tools>` list + line-53 instruction text + user turn + `<|im_start|>assistant\n`) was 490 tokens (f1), 492 with `<think>\n` appended, 494 with `<think>\n\n</think>\n\n`. Token counts differ by 2 to 6 from the native path, so the native render is not byte-identical to my hand-built one (could be tools JSON spacing/key order, or the generation-prompt suffix). Not investigated further.
- Hand-built tools JSON used Python json.dumps default separators (", ", ": "), ensure_ascii=False, tools in the order given.
- Official Jinja generation prompt ends with `<think>\n` (or `<think>\n\n</think>\n\n` when enable_thinking is false). The task's raw prompts end with `<|im_start|>assistant\n` only; in that case the model itself emitted `<think>\n\n</think>\n\n` at the start of its output (f1, f2, f3, f4, f5, g2). f1b (ends `<think>\n`) and f1c (ends `<think>\n\n</think>\n\n`) are the template-exact variants.

## Endpoint checks
- `GET /v1/models` returns `qwen3.5:9b` (i1-models). `POST /v1/chat/completions` works (j1-basic; reasoning present by default even for trivial prompts).

## Output shapes per capture
- 01-native, 05a, 05b, 05d: `message.tool_calls[0]` = read_file with `arguments` JSON string `{"path":"README.md"}`, `content` "", plus a separate `reasoning` field (not `reasoning_content`). Ids like `call_xxxxxxxx`, with `index`.
- 02-native-multi: two `tool_calls` in one message (indexes 0 and 1), both read_file, with `reasoning`.
- 03-native-no-tool: plain `content` "Paris is the capital of France.", `reasoning` present, no tool_calls, finish_reason stop.
- 04-native-unknown: no tool_calls; content text says compile_project is unavailable and lists the 4 tools; `reasoning` present.
- 05c (`reasoning_effort:"none"`): `tool_calls` present, NO `reasoning` field, 26 completion tokens vs 59.
- 05e (`/api/chat`, `think:false`): Ollama-native shape, `message.tool_calls[0].function.arguments` is a JSON object (not a string), no thinking field.
- f1..f5, g2 (`/api/generate`, raw:true): response string is raw text. Tool calls are in XML `<tool_call>\n<function=NAME>\n<parameter=KEY>\nVALUE\n</parameter>\n</function>\n</tool_call>` form. No Hermes JSON form was produced anywhere. f2: two consecutive `<tool_call>` blocks separated by a single newline, preceded by a prose sentence; f3: `content` parameter spans 3 lines with newlines preserved, closing `\n</parameter>`; f4: plain text, no tool markup; f5: plain text refusal, no `<tool_call>`. Raw mode applies no parser: `<think>` text is returned inline in `response`. f1b output has the closing `</think>` but not the opening one (opening was in the prompt).
- g1-native-result-replay: assistant message with tool_calls (id from capture 01) + `role:tool` message with `tool_call_id` and the result JSON string -> final `content` "The file `README.md` says: ..." plus `reasoning`.
- g2-raw-result-replay: raw prompt with assistant `<tool_call>` turn and a `<|im_start|>user\n<tool_response>\n{...}\n</tool_response><|im_end|>` block -> text answer quoting the result, after an empty think block.
- h1-streaming: SSE chunks; first ~30 chunks carry `delta.reasoning` tokens (one token each, `content` ""), then ONE chunk with the whole `tool_calls` entry (complete name and arguments, not fragmented), then a chunk with `finish_reason:"tool_calls"`, then `data: [DONE]`. 33 `data:` lines.
- h2: same with `reasoning_effort:"none"`: single tool_calls chunk, finish chunk, [DONE]; no reasoning chunks.

## Reasoning
- Present by default on /v1/chat/completions as `message.reasoning` (and `delta.reasoning` in streams). `reasoning_content` and literal `<think>` in content never appeared on the native path.
- Disable attempts (05b-05e, same prompt): `think:false` on /v1/chat/completions: reasoning still present. `chat_template_kwargs.enable_thinking:false`: still present. `reasoning_effort:"none"`: reasoning absent (05c, h2). `think:false` on native `/api/chat`: absent (05e).
- Raw path: reasoning is whatever the prompt prefix allows (see template section).

## Failures / caveats
- None of the requests errored. One capture mistake corrected: i1-models was first sent as a POST and was replaced by a GET capture (`request: null`, `method: GET`).
- Content is `""` (empty string, not null) when tool_calls are returned on the native path.
- Model unloads after Ollama's default keep-alive (~5 min idle); server left running.
- Outputs at temperature 0 with seed 42 were not run twice to test determinism, except that 01 and 05a (identical requests) gave identical message content/arguments (ids differ).
