# INTERPLANE 0.1 readiness report (2026-10-04)

Repository: https://github.com/aien-dev/interplane (public, Apache-2.0 core; adapters AGPL-3.0-or-later).
Format follows the mission's reporting discipline. Every claim below is backed by a file in the
repository or a command whose output is recorded there; nothing planned is described as done.

## Observed

- Neither host runtime parses Qwen3.5's official tool-call form. The official chat template
  (`qualification/evidence/qwen35-9b-chat_template.jinja`, sha256 `a4aee8af…`) emits
  `<tool_call><function=NAME><parameter=KEY>` XML; Odysseus's text parser accepts Hermes JSON only,
  and AIEN's aien-cli accepts `<tool_call>{json}</tool_call>` only (`CURRENT_STATE.md`).
- A real Qwen3.5-9B on Ollama 0.34.0 (GB10) emits that XML on the raw completion path and never
  the Hermes JSON form (`qualification/captures/qwen35-ollama/f1…f5`). On the OpenAI-compatible
  path Ollama's own parser converts it to native `tool_calls` and, when no `tools` are declared,
  swallows it entirely: content comes back empty. Reasoning is on by default as `message.reasoning`
  and only `reasoning_effort: "none"` turns it off on that path.
- AIEN has two agent stacks; its `aien-mcp` and `aien-capability` crates are merged but unused by
  the live loop, and `AuthorizedEffect` has no production constructor. aien-cli's `SafetyEngine`
  lives in a private module and cannot be linked from outside.
- Odysseus exposes 71 function tools; its gate (`ToolRunSecurityContext.decision_for`) arms after
  any workspace read and then requires approval for effectful tools; `tail_serve_output` is in the
  schema list but missing from `TOOL_TAGS`, so its own native-call path rejects it.
- Ajax: no official artifact exists as of 2026-10-04. Nothing about Ajax was fabricated; the dialect
  slot is reserved and refuses to register.

## Changed

- Normative spec: 10 JSON Schema 2020-12 documents, `spec/CORE.md` (ids, limits, lifecycle,
  pinned result shape and error messages, JCS digests), `LENSHIFT.md` (openai, qwen35, aien_legacy,
  ajax reserved), `CROSSVEIL.md`, `CROSSAXIS.md`, `PROBE.md`, `VECTORVEIL.md`, `RELAYLINE.md`,
  `VERSIONING.md`; ADRs 0001 (scope), 0002 (licensing), 0003 (AIEN adapter mints nothing).
- Conformance corpus: 23 cases plus a JCS digest fixture; 31 dialect fixtures across three dialects.
- Reference implementations: Rust (6 crates, 84 tests) and Python (stdlib-only package, 175
  tests), each with a conformance runner and a probe.
- Adapters: Odysseus (Python, 55 tests against odysseus@2992bf6) and AIEN (Rust, 10 pipeline tests
  against aien-sovereign-core@7580039 + aegis-runtime@f4e8709).
- Qualification: real Qwen3.5-9B captures, two live probe reports (Rust and Python), and the
  before/after receipt (`examples/receipt`).
- CI: schema self-check, fixture validation, Rust and Python suites, cross-language verdict
  identity (23 cases), both adapters against pinned upstream checkouts.

## Proven

- Rust and Python produce byte-identical conformance verdict files, including every result digest,
  on all 23 cases (`cmp` on the two JCS outputs; CI job `cross-language`).
- The mock-runtime proof: allowed read executes; denied write never reaches `execute`; malformed,
  oversized, duplicate, replayed, unknown-version and unknown-decision inputs fail closed; a model
  retry after denial is decided again by the runtime and denied again (cases 01–23).
- Through the real Odysseus code: an authorized `read_file` executes via Odysseus's own handler
  under Odysseus's path confinement; a workspace escape is refused before execution; `send_email`
  after a read is `requires_approval`; a non-admin blocked tool is `denied`; an unknown name is
  `not_found`; malformed arguments are `invalid`; with Odysseus absent every decision is `denied`.
- Through the real AIEN crates: reads run through `SpeculativeLane::invoke_speculative`; effectful
  requests are staged with `stage_effect_intent` and come back `requires_approval` with `execute`
  never invoked; the aegis `pre_dispatch_check` gate denies an unlisted shell command; absent AIEN
  fails closed.
- The 0.1 goal receipt (`examples/receipt/receipts/qwen35-9b-ollama-run1.json`, `run2.json`):
  same Qwen3.5 task over the Odysseus catalog, 71 tools vs 9 selected by CrossAxis
  `domain_match`; first-turn prompt tokens 15 846 vs 1 674 (89.4% fewer); both runs reached the
  correct answer; every tool call was decided by Odysseus and executed only through Crossveil;
  run 2 reproduced run 1's deterministic digest, answers and token counts.
- Probe on Qwen3.5-9B/Ollama 0.34.0: 13 PASS, 1 UNSUPPORTED (`tools.text_qwen35`), identical
  verdicts from the Rust and Python probes; profiles `interplane.core.0.1`, `lenshift.openai.1`,
  `relayline.tool_replay.1` compatible, `lenshift.qwen35.1` incompatible on that endpoint.

## Failed

- `lenshift.qwen35.1` is incompatible on Ollama's OpenAI path: the backend consumes the text-form
  call. This is a backend property, measured, not a protocol failure; the raw path proves the model
  itself emits the form.
- In the full-surface receipt run the model chose `get_workspace` and then `bash`, which the
  reference Odysseus adapter does not execute (`execution_error`, by design). The model recovered;
  the receipt records the failures rather than hiding them.
- First probe attempt failed `chat.basic` because a 64-token budget was consumed by default
  reasoning. Fixed by budget; the finding is recorded in the probe's detail text.

## Unproven

- Rust and Python agreement on `duration_ms` (excluded from digests by spec) and on probe reports'
  exact bytes (verdicts agree; detail strings and timings differ by design).
- Any claim about Ajax.
- Native Qwen3.5 text-form parsing against vLLM, llama.cpp or SGLang (only Ollama was measured).
- Task-success regression across a 30–50 task benchmark (Bench is 0.5 work; one task was measured).
- Approval round-trips: both adapters mint only opaque pending handles; nothing consumes them yet.
- The AIEN adapter's read provider is adapter code behind AIEN's `MemoryWire`; AIEN ships no
  enrolled catalog today, so the four descriptors are the adapter's, built from AIEN's own
  effect bits.

## Architectural impact

- Authority never moved: no INTERPLANE component constructs `authorized` except the mock runtime,
  and the Rust lifecycle's state field is private. Both adapters translate host decisions only.
- Two-axis trust (content kind × trust level, `unknown` explicit) is on the wire now so 0.3 does
  not change the format.
- The pinned result shape and error messages in `CORE.md` are what make two implementations agree
  byte-for-byte; the one divergence found (provenance of decided-but-not-executed results) was
  pinned in the spec rather than papered over in code.
- CrossAxis Select is a deterministic function of catalog + domains; it reduces exposure, it never
  grants. The receipt shows the practical effect on a small local model.

## Upstream impact

- Odysseus: one narrow fix prepared for issue #6474 (`llama-server -hf … --hf-file …`, with a
  regression test) on branch `fix/cookbook-llamacpp-hf-file`; per their CONTRIBUTING rule an AI
  agent must not open the PR, so the branch and PR text are ready for a human. Findings worth
  reporting upstream: Qwen3.5 XML form unparsed; `tail_serve_output` missing from `TOOL_TAGS`.
- AIEN: `SafetyEngine` is private (ADR 0003 assumed it could be linked); `aegis-runtime` cannot be
  consumed as a git dependency because it reaches a sibling checkout by path; no crate enrolls a
  tool catalog into aien-mcp today. Each is a small upstream change, none is required for 0.1.

## Next step

0.2 (Capability): token accounting with a real tokenizer in `selection.measure`, the task-success
regression suite (30–50 tasks over the Odysseus catalog), and `tools.text_qwen35` measured on
llama.cpp and vLLM so the Qwen3.5 dialect's profile can be qualified somewhere real.
