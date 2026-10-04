# Qwen3.5 9B on SGLang: captured outputs and qualification (2026-10-04)

Facts only. Each `NAME.json` holds `request` (exact body), `endpoint`, `status`, `response` (exact raw body; for streams the SSE text), `sha256_response`. Requests are the Ollama capture bodies (qualification/captures/qwen35-ollama) with only `model` changed, temperature 0, seed 42, same 4-tool set. Names mirror the Ollama set; extras: `*-noparser` (server relaunched without `--tool-call-parser`), `f*-completions-*` (raw prompt via `/v1/completions`, replaces Ollama `/api/generate raw`), `f*-chat-text-*` (the same instruction as a chat system message, no `tools`), `k1`, `k2` (the Rust probe's own prompts).

## Environment
- DGX Spark GB10 (compute capability 12.1), driver 580.173.02, CUDA 13.0, Linux 7.0.0-1019-nvidia aarch64. Host has no pip SGLang; Option 2 used.
- SGLang: Docker `lmsysorg/sglang:latest-cu130` (multi-arch, arm64 variant pulled), image digest sha256:06e4f2ed21afde4ff513cda65070124e727ba23ccaeff7712b8c40e1097d611f, in-image `sglang 0.5.20`, torch 2.13.0+cu130 (CUDA 13.0). Attention backend auto-selected: flashinfer. No source build, no resolver problems (Option 1 was not attempted).
- Model: `Qwen/Qwen3.5-9B` safetensors bf16, HF revision c202236235762e1c871ad0ccb60c8ee5ba337b9a, 4 shards, 19 GB on disk (`hf download`). Official `chat_template.jinja` is used (no custom template).
- Launch: `docker run -d --name sglang-qwen35 --gpus all --network host --ipc=host -v ~/.cache/huggingface:/root/.cache/huggingface lmsysorg/sglang:latest-cu130 python3 -m sglang.launch_server --model-path Qwen/Qwen3.5-9B --port 18082 --host 127.0.0.1 --reasoning-parser qwen3 --context-length 32768 --mem-fraction-static 0.5 --tool-call-parser qwen3_coder`.
- Parser names offered by this build: tool-call `{auto,apertus2509,cohere_command4,deepseekv3,deepseekv31,deepseekv32,deepseekv4,dots,glm,glm45,glm47,gpt-oss,k2_horizon,kimi_k2,kimi_k3,lfm2,ling3,llama3,mimo,minicpm5,mistral,muse,poolside_v1,pythonic,qwen,qwen25,qwen3_coder,spark25,step3,step3p5,minimax-m2,minimax-m3,nanbeige,trinity,interns1,hermes,hunyuan,gigachat3,gemma4,inkling}`; reasoning `{...,qwen3,qwen3-thinking,...}`. There is no `qwen35` name; `qwen3_coder` was chosen because it is the XML `<function=...><parameter=...>` form the Ollama captures show Qwen3.5 emitting. `auto` and the other names were NOT tried (UNVERIFIED which of them also work).
- Other jobs were on the GPU during runs (Ollama, a llama.cpp lane): decode was ~7 tok/s at times, so durations in reports are not a benchmark.

## Probe results (reports/qwen35-9b-sglang-0.5.20.probe.json and .probe.rust.json)
Python: 14/14 PASS, all four profiles compatible. Rust: 12 PASS, `chat.basic` FAIL, `json.structured` FAIL; `interplane.core.0.1` incompatible, the other profiles compatible. Verdicts differ because the two probes send different prompts, not because of a harness bug: the Rust chat.basic prompt ("Reply with one short sentence about the sea.", max_tokens 2048) makes Qwen3.5 think for the whole budget (k1: finish_reason length, content "", 2048 reasoning tokens). Same for the Rust JSON prompt (k2). Python's prompts ("Reply with the single word: ready", "...answer set to 42") finished within budget. A rerun of the Rust pair reproduced both FAILs (first Rust run timed out on json.structured at 300 s under GPU contention; rerun at 900 s got "content is not valid JSON"). Probe is descriptive; the server was not modified between probe runs.

## Matrix (same-prompt captures, parser on unless stated)
| Capability | Verdict | Evidence |
|---|---|---|
| chat.basic | PASS (Python) / FAIL (Rust, reasoning budget) | j1-basic; k1 |
| tools.openai (native tool_calls) | PASS | 01-native: one tool_call read_file `{"path": "README.md"}`; ids `call_<24hex>`; content `"\n\n"` |
| tools.parallel | PASS | 02-native-multi: 2 tool_calls |
| tools.text_qwen35 | PASS: XML passes through as content, not converted | f1-chat-text-one-call, f1-completions-one-call; same with parser off (`-noparser`) |
| tool replay | PASS | g1-native-result-replay (final answer quotes result); g2-completions-result-replay |
| streaming | PASS, fragmented tool call | h1-streaming: 48 reasoning chunks then 4 tool_call chunks (name with empty arguments first, then argument pieces), finish `tool_calls`; h2 (reasoning_effort none): 6 chunks, no reasoning |
| unknown tool | PASS (no tool call) | 04-native-unknown: prose listing available tools |
| malformed args | FAIL-CLOSED at request: HTTP 400 "Assistant tool call function.arguments must be valid JSON." | m1-malformed-arguments-replay (a replayed assistant call with invalid JSON; model-side malformed output was not elicited, UNKNOWN) |
| reasoning | PASS, field `reasoning_content` | 05a (not `reasoning` as on Ollama) |
| reasoning.disable | PASS via `chat_template_kwargs.enable_thinking=false` | 05d, 05d2; also `reasoning_effort:"none"` works (05c, h2); `think:false` ignored (05b still reasons); `separate_reasoning:false` does NOT disable, it moves reasoning text into `content` (05e); with enable_thinking false plus separate false: no reasoning (05f) |
| json.structured | PASS (Python) / FAIL (Rust, reasoning budget) | probe reports; k2 |
| context 8k/16k/32k | PASS (Python and Rust) | probe reports |
| models.list | PASS | i1-models |

## Behavior specific to SGLang (not Ollama/llama.cpp)
- With `--reasoning-parser qwen3` the answer `content` begins with `"\n\n"` (03: `"\n\nThe capital of France is Paris."`), and tool-call responses have content `"\n\n"`, not `""`.
- Tool-call parser only acts when the request has `tools`. Without `tools` (f1-chat-text) the `<tool_call>` XML stays in content even with the parser on. With the parser off and `tools` present, the XML also stays in content and `tool_calls` is null (01-native-noparser, 02-native-multi-noparser).
- Raw `/v1/completions` returns the model's own `<think>\n\n</think>\n\n` prefix inline (f1-completions); with the prompt ending `<|im_start|>assistant\n` the model sometimes opens `<think>` itself (g2).
- f5 (unknown tool asked in raw form) differs per path: completions emitted a `list_dir` XML call after prose; chat text refused in prose.
- Determinism across repeated runs was not tested.

## Limits
- Only `qwen3_coder` was tested for tool parsing; not the `qwen`/`qwen25`/`hermes`/`auto` names.
- bf16 weights, not the Q4_K_M of the Ollama capture, so differences vs Ollama mix engine and quantization effects.
- Server left running: container `sglang-qwen35`, host port 127.0.0.1:18082 (container PID 290206 at the time of writing, with `--tool-call-parser qwen3_coder`).
