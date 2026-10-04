# Vectorveil (v0.1: the envelope only)

Vectorveil owns packaging and transport. In 0.1 it is exactly `schemas/envelope.schema.json`,
the id rules in `CORE.md`, and JCS canonical JSON for digests. Transport is in-process or ordinary
request/response; no broker, no RPC framework.

Reserved, not implemented: `signature` (origin proof, never authority), remote transport bindings.
Adding either must not change the envelope's required fields.

Typed failure envelopes (degraded-state reporting): a `result` with `status: error` and
`error.code` from `ErrorCode` plus `error.details.state` in
`{unavailable, degraded, timeout, auth_required, misconfigured, unsupported}` is how a runtime or
probe reports a backend that is not healthy. `runtime_unavailable` is the code; `details.state` is
the shape. This is the only Vectorveil addition in 0.1 and it is schema-compatible today.
