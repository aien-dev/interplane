# Upstream PR draft (NOT SENT: Drake's button). Targets PlummersSoftwareLLC/NightDriverStrip main.

Branch: aien-dev/NightDriverStrip `fix/565-buffer-budget-8bit` (8e3d3515), based on main `4d79c290`.
CONTRIBUTING.md exists on main and has no AI-assistance disclosure requirement (grep for AI, LLM, assisted, generated: no hits). Disclosure is still recommended by courtesy: "Drafted with AI assistance (Claude); reviewed and tested by the submitter." Add only if Drake agrees.
Nothing in this text may mention INTERPLANE or AIEN.

## Title
Size the LED buffer budget from 8-bit internal memory on boards without PSRAM

## Body
`SetupBufferManagers()` computes how many LED buffer sets to allocate from `ESP.getFreeHeap()` when there is no PSRAM. That is `heap_caps_get_free_size(MALLOC_CAP_INTERNAL)`, which on the original ESP32 also counts the IRAM heap. IRAM is 32-bit access only (no `MALLOC_CAP_8BIT`), and the buffers are allocated with `MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT` (`psram_allocator` fallback), so that memory can never serve them. The configured `RESERVE_MEMORY` can therefore end up smaller than intended.

This adds `FreeInternalBytes8Bit()` (`heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT)`) and uses it for the non-PSRAM budget. PSRAM builds are unchanged.

What this is not: it does not claim to fix #565. That report has no failing call site or capability, and the numbers in #562 (largest block shrinking by exactly one 4 KiB stack allocation) suggest ordinary shortage there. This only removes one place where the budget was computed from a different kind of memory than the allocation uses.

Behaviour change: on classic ESP32 without PSRAM the buffer count can drop by roughly (free IRAM / bytes per buffer set). Boards that now fall below `MIN_BUFFERS` will refuse at startup, as they would have had the memory really been short.

Testing: not compiled for an ESP32 target and not run on hardware by the author. Logic checked on a host against a model of the documented ESP-IDF capability rules only. Needs a maintainer or tester with an M5StickC Plus (or any no-PSRAM ESP32) to confirm it builds and boots, and to compare the logged "Reserving N LED buffers" line before and after.

References: ESP-IDF v5.5.2 `components/heap/heap_caps.c:277-286,367-389`, `components/heap/port/esp32/memory_layout.c:54`, `components/heap/heap_caps_base.c:147`; arduino-esp32 3.3.7 `cores/esp32/Esp.cpp:163-173`.

## Suggested comment for #565 (separate, also unsent)
A reproducer needs the failing allocation's size and caps. ESP-IDF can report them: `heap_caps_register_failed_alloc_callback` (heap_caps.c:62-70). Printing `heap_caps_get_largest_free_block` for `INTERNAL|8BIT` instead of `INTERNAL` would also show whether the "largest block" in the logs is actually usable for 8-bit allocations. Without those two numbers the "4K failed with 34K largest" case cannot be told apart from an IRAM-counted block, a DMA-capable requirement, or a measurement taken at a different moment than the failure.
