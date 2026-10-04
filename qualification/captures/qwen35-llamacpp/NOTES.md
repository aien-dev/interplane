# Qwen3.5 9B on llama.cpp: captured outputs (2026-10-04)

Facts only. Each capture JSON holds `request`, `endpoint`, `http_status`, `response` (raw body string; streams are the concatenated SSE text) and `sha256_response`. Same request bodies as `../qwen35-ollama/` (temperature 0, seed 42; max_tokens 4096 added). Exact build and launch in `launch.txt`; scratch capture script in `capture_script.py.txt`.

## Environment
- Host NVIDIA DGX Spark GB10, Linux aarch64 7.0.0-1019-nvidia, 128 GB unified memory. CUDA toolkit 13.0.88.
- llama.cpp tag b11398, commit a7b94df2c616bc1f62a73b964b4a71cb0dcc488e, built from source with CUDA (arch 121), `-ngl 99`, `-c 32768`, `--jinja`. Server on 127.0.0.1:18081, 4 slots, model id `qwen3.5:9b` (via `--alias`).
- Model: the Ollama blob for `qwen3.5:9b`, used directly with `-m` (sha256 02d45dc1cf451ba2475ac33b301c2dd8f985abe4c182ce04a1f2f5bf0260278d, 5,618,141,184 bytes per /v1/models, Q4_K - Medium, 8.95 B params, n_ctx_train 262144, vision disabled (no projector loaded)). Same weights as the Ollama run, so engine differences are not weight differences.
- Ollama was left running and untouched.

## Template
- llama.cpp used the template embedded in the GGUF. `GET /props` returns it (`chat_template.jinja`); its sha256 a4aee8af... equals the official Qwen/Qwen3.5-9B chat_template.jinja saved in `qualification/evidence/`. So llama.cpp (unlike Ollama's Go renderer) renders from the official Jinja. /props `chat_template_caps`: tools, parallel tool calls, object arguments supported; reasoning_effort NOT a declared template capability (yet `reasoning_effort:"none"` disabled thinking, see below).
- `/apply-template` (t0) shows the prompt ends with `<|im_start|>assistant\n<think>\n` (generation prompt opens thinking).

## Output shapes
- Native (01, 02, 05a): `tool_calls` with `arguments` as JSON string, id is a 32-char random string, `content` "", reasoning in `message.reasoning_content` (Ollama used `reasoning`). Two independent calls come back as two `tool_calls` (02).
- 03: plain content, finish_reason stop, reasoning present. 04: no tool call, text lists the available tools.
- g1 (role tool replay): final content quotes the result.
- h1 streaming: reasoning arrives as `delta.reasoning_content` chunks, then the tool call arrives FRAGMENTED over several chunks (first chunk has id+name+`{`, later chunks add argument pieces), unlike Ollama which sent one whole chunk. Clients must accumulate by `index`.
- f1/f2/f5 (no `tools` field, instruction in the system prompt, via /v1/chat/completions): llama.cpp does NOT convert; the `<tool_call><function=...><parameter=...>` text comes back verbatim in `content` (reasoning still split into `reasoning_content`, since the generation prompt opens `<think>`). Same via `/completion` raw (f1r, f2r, f5r), where the model's own `<think>...</think>` stays inline in `content`. Two calls = two consecutive blocks separated by a newline (f2). f5: plain-text refusal, no block.
- Quirk: when `tools` IS sent (native), llama.cpp parses the same XML into `tool_calls`; when not, it passes it through.

## Reasoning
- Default: reasoning present (`reasoning_content`), even for "Say hello" (j1: 367 completion tokens for "Hello!").
- Disable, same prompt (05a-05f, plus server flags):
  - `chat_template_kwargs.enable_thinking:false` (05d): disabled (0 reasoning chars, 26 tokens).
  - `reasoning_effort:"none"` (05c): disabled (26 tokens).
  - `think:false` (05b): ignored, reasoning still present (same as Ollama on /v1).
  - `reasoning_format:"none"` (05f): not a disable; `<think>...</think>` moves into `content`.
  - server flag `--reasoning off` or `--reasoning-budget 0` (temporary server): disabled, tool call still returned.

## Malformed arguments
- m1 (prompt asks for the literal string "very loud" for an integer parameter): model declined in prose, no tool call. Inconclusive for the engine.
- m1b (schema integer, description says it is a preset name, prompt "set the loud preset"): the model's raw output (m1d, via /apply-template then /completion) is `<parameter=level>\nloud\n</parameter>`, but llama.cpp returned `tool_calls[0].function.arguments = {"level":1}`: it silently coerced the invalid string to an integer. m1c (`tool_choice:"required"`) gave the same. So llama.cpp does not surface a string where an integer is declared and does not report an error; the value is changed. (Why "loud" became 1 was not investigated; UNVERIFIED mechanism.)

## Tokenize
- t1-tokenize: `POST /tokenize` with `{"content": "<string>", "add_special": false, "with_pieces": false}` returns `{"tokens": [int, ...]}`. Works on a compact-JSON dump of the 4-tool schema. `POST /apply-template` with `{"messages":[...],"tools":[...]}` returns `{"prompt": "<rendered chat text>"}` (t0). Together these give exact prompt-token counts for a tool schema; usage.prompt_tokens in responses gives the same count for a full request.

## Matrix (probe vocabulary)
| Row | Verdict | Evidence |
|---|---|---|
| chat.basic | PASS (Python and Rust probes, after alignment) | reports; see Probe agreement |
| tools.openai (native) | PASS | 01-native.json |
| tools.text_qwen35 (raw form) | PASS, passthrough in content, not converted | f1-raw-one-call.json, f1r |
| reasoning | PASS, field `reasoning_content` | 05a, j1 |
| reasoning.disable | PASS via enable_thinking=false and reasoning_effort=none; think=false UNSUPPORTED (ignored) | 05d, 05c, 05b |
| tool replay | PASS | g1-native-result-replay.json |
| streaming | PASS, tool_calls fragmented across chunks | h1-streaming.json |
| unknown tool | PASS (no call to a nonexistent tool) | 04-native-unknown.json, f5 |
| malformed args | FAIL for strictness: invalid string silently coerced to integer 1 | m1b, m1d |
| parallel calls | PASS | 02-native-multi.json |

## Probe agreement (Python vs Rust)
Reports: `qualification/reports/qwen35-9b-llamacpp-b11398.probe.json` (Python) and `.probe.rust.json` (Rust).
- Previous state (first Rust run): all probes agreed except `chat.basic`: Python PASS, Rust FAIL ("empty content; reasoning channel consumed the completion budget"; finish_reason length, 2048 completion tokens, 7144 reasoning chars, empty content). Rust profile `interplane.core.0.1` showed incompatible, Python compatible.
- Cause: the two probes sent different requests. Python: "Reply with the single word: ready" (the Python endpoint helper adds temperature 0 and seed 42; the earlier note that no temperature was set was wrong). Rust: "Reply with one short sentence about the sea." at temperature 0, no seed. At temperature 0 the thinking on the Rust prompt ran past 2048 tokens. The same split existed in every other probe (Rust used a get_weather tool and other prompts, max_tokens 2048 on every request, no seed; Python used read_file/list_dir, max_tokens only on chat/streaming/context, seed 42, a different context needle and prompt builder), and the Rust `models.list` request digest hashed a JSON object where Python hashes the line "GET /models".
- Fix: the Rust probe now builds the same bodies as the Python probe (spec/PROBE.md: both reference probes send identical request bodies per probe id). Python is unchanged and is the reference. Verdict logic is unchanged in both. Unit tests pin the chat.basic body in both languages (same JCS digest sha256:ed2eb0d6...) and the context prompt lengths.
- Rerun (2026-10-04, llama.cpp b11398 on 127.0.0.1:18081, same server, not restarted): Rust report overwritten; the Python report is the earlier one (Python requests did not change). Result: all 14 probes agree (every verdict PASS in both), all four profiles compatible in both. Request digests are equal per probe id for 13 of 14. The one differing digest is `tools.result_replay`: its body embeds the tool call (a random id) that each run's own `tools.native` response produced, so the digests differ by construction while the body shape is the same.
- Limit: single run per probe on a model that burns thinking tokens; at temperature 0 with seed 42 results are expected to repeat but no repeat run was made.

## Caveats
- The tools.text_qwen35 PASS needs no engine support (it is plain text passthrough).
- Single runs per capture; no repeat-determinism test.
- Ports: a temporary second server used port 18083 (18082 is occupied by another service).
