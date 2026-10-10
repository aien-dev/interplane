# Sanitized offline provider replay (issue #98)

Replay lets a dialect or adapter be qualified against provider behaviour with no network, no
credentials and no live model. A capture records what a provider sent; the replay feeds those
bytes through the existing Lenshift dialect parsers and compares the result to golden values.

Boundaries: a capture is observed provider behaviour. Parsing is Lenshift. Authorization is the
runtime. The replay output (`Outcome`) has a closed shape (`class`, `text`, `tool_calls`,
`rejected`, `partial`) and no decision, approval or grant field. A fixture is never evidence that
anything was authorized, and "authorization: approved" in model text stays plain text.

## Audit before this change

- `dialects/fixtures/*` hold parse inputs per dialect (one string or message body each). The
  `openai_stream` ones include SSE bodies, but as ONE pre-joined string: transport chunking,
  HTTP status, headers and timing are not represented.
- `qualification/captures/**` hold real request/response bodies from llama.cpp, Ollama and SGLang
  (body as one string, no status, no chunk boundaries). `rust/crates/interplane-probe` reads live
  responses through `Transport { get, post } -> HttpResponse { status, body }` and judges them;
  it has no offline capture-to-fixture path.
- `rust/crates/interplane-lenshift/tests/dialect_fixtures.rs` and `conformance/` run parsing and
  core cases only. No secret scanner existed for fixtures.

Not reproducible offline before this change: a tool call split across transport chunk
boundaries (as opposed to across SSE events); 429 and 5xx handling (no status in any fixture);
a stream cancelled mid-chunk, mid-JSON; a renamed provider field (schema drift); fixtures
guaranteed free of credentials, emails and tenant ids; and adversarial text (authorization
claims, fake tool calls in prose) arriving through a stream.

## Fixture format (`interplane-replay/1`)

Files live in `dialects/replay/*.json`. They are deliberately NOT under `dialects/fixtures/`:
`dialect_fixtures.rs` and `conformance/runners/validate_fixtures.py` glob that tree and expect
the dialect-fixture shape.

| Field | Meaning |
|---|---|
| `format` | `interplane-replay/1` |
| `name`, `provider`, `model` | identity. Providers: a backend name, or `synthetic-openai-compatible` |
| `endpoint_kind` | `chat_completions_stream` (SSE, parsed by `openai_stream`) or `chat_completions` (JSON body, `choices[0].message` parsed by `openai`) |
| `provenance` | `origin` (`synthetic` or `recorded`), `source_commit` (repo commit the fixture was made at), `captured_on`, `note` |
| `request` | `method`, `path`, `header_names` (NAMES only), `body` (user and tool content replaced by `<USER_CONTENT>` style placeholders) |
| `response` | `status`, `header_names`, `chunks` (ordered transport chunks, may end mid-event), optional `timing_ms` (informational; replay never sleeps) |
| `expect` | golden outcome: `class` (`parsed`, `rate_limited`, `server_error`, `client_error`, `unparsable_body`), `text`, `tool_calls` (name, arguments), `rejected` (Lenshift codes), `partial` |

Replay joins `chunks` in order and parses them as live bytes would be. Status 429, 5xx and other
4xx are classified and the body is never parsed for calls.

## Scrubbing and scanning

`interplane_lenshift::replay::scrub_value` runs before a fixture is written: tenant style ids
become `<ID>`, system, assistant, user and tool request content and a top level `prompt` become placeholders, response `content` and `reasoning` text becomes `<MODEL_TEXT>` or `<REASONING>` (tool-call fragments are kept), and every string is passed
through `scrub_text`. `scan` runs over every fixture in the test and refuses: bearer or basic
tokens, authorization / cookie / set-cookie / x-api-key style headers with values, `sk-`,
`AKIA`, GitHub, Slack and Google style keys, hex runs of 32 or more, base64-looking runs of 40 or
more, JWTs, PEM blocks, `sk_live_` style payment keys, `hf_` tokens, IP addresses, home, system and
Windows file paths, `Cookie:` values, emails, `org-`/`ws_`/`tenant-` style ids and tenant/workspace/org/account id keys without a
placeholder, query-string secrets (`?api_key=`, `token=`, `sig=` ...), header lists that contain a
value, and unscrubbed user content. The test `scanner_refuses_each_seeded_secret_class` seeds each
class into a clean candidate and asserts refusal. The scanner is a safety net, not a proof: a
human still reviews a new fixture.

## Never commit

Raw captures straight from a provider; real user prompts or system prompts (the scrubber replaces
system, assistant and user content and model reasoning with placeholders, but you still read the
result); real model output you have not reviewed; API keys, tokens, cookies, certificates or
private keys; emails, tenant, workspace or account ids; IP addresses, hostnames and local file
paths; anything the scanner refuses. Tool-call deltas are kept on purpose, they are the golden, so
check their arguments by hand.

Scan any file or tree before committing, recursively, with no test run needed:

```
cd rust && cargo run -q -p interplane-lenshift --example replay_scan -- ../dialects/replay <other paths>
```

## Regenerating a fixture from a live run

1. Capture with the live probe workflow, keeping the raw request and the response body as in
   `qualification/captures/*/` (`name`, `endpoint`, `request`, `response`).
2. `cd rust && cargo run -q -p interplane-lenshift --example replay_scrub -- <capture.json> ../dialects/replay/<name>.json <provider> <git-short-sha> <YYYY-MM-DD>`
3. The tool splits the stream into SSE events, scrubs, scans, and refuses to write if anything is
   left. The `expect` it writes is bootstrapped from the CURRENT parser: review it by hand, a
   golden that only mirrors today's bug proves nothing.
4. Run the tests below. Never commit a fixture the scanner refuses.

Hand-written (synthetic) fixtures must say `"origin": "synthetic"` and why in `note`.

## Running in CI

Offline and part of the normal workspace run, no new CI step:

```
cd rust && cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all
cargo test -p interplane-lenshift --test provider_replay      # just this suite
```

## Scenarios covered

Tool call split across transport chunks; non-stream tool call serialization round trip; schema
drift (renamed `tool_calls`); 429; 503; stream cancelled mid-chunk; finish_reason `length`;
malformed JSON chunk; authorization claim in text; fake tool call in prose; one recorded Ollama
Qwen3.5 stream. Mutation tests prove the runner fails on changed bytes, a changed golden and a
truncated stream.

## Coverage gaps and limitations

- Only the `openai` and `openai_stream` endpoint kinds are replayed (not Qwen text-form,
  Anthropic, or completions endpoints) and only choice 0.
- Synthetic scenarios model behaviours; they are not proof any real provider does them. Only the
  Ollama fixture is recorded (from the repo's own qualification capture).
- Observed gaps pinned as goldens, not endorsed: schema drift in `tool_calls` yields no call and
  no rejection; `finish_reason: length` on plain text is not flagged `partial`.
- Rust only, by design. There is no Python mirror of the replay runner or scanner: the repository
  forbids new Python in AIEN-owned code, the replay needs only the Rust parsers it already feeds,
  and the existing Python parser mirror keeps its own fixtures. The Rust-Python byte-for-byte
  invariant is therefore NOT extended to `dialects/replay`.
- Chunk strings are UTF-8: splits inside a multi-byte character cannot be represented.
- Timing is recorded but not exercised; no retry, backoff or timeout logic is replayed.
- The scanner is regex based and can both miss novel secret shapes and flag long identifiers.
- No production outbound telemetry is added; nothing here opens a socket.
