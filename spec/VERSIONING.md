# INTERPLANE versioning

Protocol version is `MAJOR.MINOR` and appears in every envelope as `interplane_version`.

| Change | Effect on version | Receiver behaviour |
|---|---|---|
| New optional field | MINOR | Older receivers ignore it. It must be safe to ignore. |
| New enum value in a non-authority enum (`event`, `ErrorCode`, `Origin`) | MINOR | Receivers tolerate unknown values where the spec says so. |
| New value in an authority enum (`decision`, `TrustLevel`) | MAJOR | Unknown decision values are treated as `denied`. Unknown trust levels are treated as `external_untrusted`. Never default to authorization. |
| Required field added or removed, field type changed, semantic change | MAJOR | Receiver rejects the envelope with `unsupported_version` if MAJOR differs from what it implements. |
| Dialect parser change that alters canonical output for an existing fixture | parser_version MAJOR (per dialect) | Recorded in `provenance.parser_version`. Fixture expectations are re-pinned. |
| Mapping table change (CrossAxis) | `mapping.table_version` | Receipts carry the version; old receipts stay interpretable. |

Rules:

1. A receiver implements exactly one MAJOR. Any other MAJOR is rejected before parsing the payload.
2. Unknown top-level fields and unknown `extensions` keys are preserved or ignored, never acted on.
3. Unknown `payload.kind` is rejected with `malformed_envelope`.
4. Fixtures under `conformance/` are the compatibility contract; a change that alters an expected
   outcome is a MAJOR change unless the fixture was wrong, in which case the CHANGELOG says so.
5. During 0.x, MINOR may still break; the fixture suite is the thing that is stable.
