# RelayLine (v0.1: the event contract only)

RelayLine defines how an exchange continues across the boundary. 0.1 defines the event vocabulary
(`schemas/event.schema.json`) and ordering; it does not implement streaming.

Per trace, `seq` is monotonic. Legal order for one request:
`turn_started` -> `tool_requested` -> (`mapped`) -> `decided` -> (`execution_started` -> `result`) -> ...
-> `model_continuation` | `final` | `cancelled` | `error`.

Reference pipelines emit these events into an in-memory list so a caller can observe them; the
conformance runner does not compare events across languages in 0.1 (observed records are the
contract). Streaming, partial tool calls and cancellation are documented inputs
(`LenshiftTurn.partial`) but not transported. Formalization waits for measured behaviour of the two
real runtimes (Odysseus SSE loop, aegis-runtime gateway).
