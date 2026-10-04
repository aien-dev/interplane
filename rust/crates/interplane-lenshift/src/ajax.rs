//! `ajax` is reserved and NOT registered. No official artifact (weights, model card, chat
//! template) existed on 2026-10-04, so no parser exists. Adding one means a new module like this
//! one, a `registry.register(...)` call, and fixtures captured from the official artifact; no
//! Core change. The registry accepting new dialects without touching `interplane-core` is the
//! abstraction test.
