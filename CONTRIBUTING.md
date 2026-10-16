# Contributing

- Small conventional commits (`feat(core): ...`, `test(conformance): ...`). One conceptual layer per PR.
- Schemas and fixtures are normative. A change that alters an expected fixture outcome is a protocol
  change and needs a CHANGELOG entry and, during 0.x, a MINOR bump.
- Every protocol object needs positive, negative, round-trip, version, unknown-field and malformed-input tests.
- Every dialect needs real fixtures. Model-dependent tests are integration tests, off by default.
- No Python. The Rust conformance verdicts and negative-control matrix must equal `conformance/expected/`
  byte for byte; CI checks both. A change that alters them updates those files in the same PR.
- No execution authority in Core. No host-project dependency in Core. No serving-engine dependency anywhere.
- Update `STATUS.md` in the same PR as the code it describes.
