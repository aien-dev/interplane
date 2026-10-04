# NightDriverStrip issue 565: "Memory allocations fail in low memory situations, but too early"

Lane D2. Upstream project: PlummersSoftwareLLC/NightDriverStrip. Nothing here has been sent upstream
(`upstreamed` is not claimed anywhere below). Pinned main: `4d79c290` (same SHA as lane D1).

**Outcome: root cause NOT proven; one verified capability mismatch in a size decision (hardening, not a claimed fix for #565); real reproduction BLOCKED on a physical M5StickC Plus.**

Evidence tags: observed, reproduced, inferred, hypothesized, fixed, tested, upstreamed, accepted.
"Model" means a host simulation of documented ESP-IDF semantics, never the real allocator.

## Observed

The issue (Dave Plummer, 2023-12-10), complete text of the claim:
- [observed] "Even allocating 4K when 50K is free, and 34K is the largest block, can fail." Mostly on builds "like M5DEMO where there is no PSRAM". No call site, no caps, no log, no firmware version. Two comments: robertlipe asks for a reproducer and suspects `String` fragmentation (guess, no profile); rbergen points at discussion #562.
- [observed] Discussion #562 is the only thread with numbers (M5StickC Plus, FANSET; discussioncomment-7934601 and the replies):
  - M5StickC Plus has no PSRAM (robertlipe, 2023-12-24). The `m5demo` env of that era extends `dev_m5stick-c`, platform `platformio/espressif32@^6.3.0` (`platformio.ini` at `960daf50`, lines 239 and 322-326). So the chip is the classic ESP32.
  - Boot log: "Reserving 49 LED buffers for a total of 29792 bytes". Heap at "Launching Drawing Thread": Mem 136412, LargestBlk 90100. At "Launching Remote Thread": Mem 123576, LargestBlk 86004 (a drop of exactly 4096). After WiFi starts: Mem 70692, LargestBlk 36852; then Mem 44532, LargestBlk 31732.
  - rbergen: "With all other servers disabled there's 47K of RAM left, with the reported biggest block being 31K. That's just not enough to start the webserver successfully."; later: lowering MAX_BUFFERS to 5 fixed the reboots on WiFi connect; "bad_alloc" in the log is "a dead give-away"; a dependency update "chewed up ~30K"; free RAM "hovers around 55K" around the crashes.
  - Two non-allocator bugs in that same thread were fixed separately (FireFanEffect negative index; JSON halving of LED count on every reboot). Not part of #565.
- [observed] No failing allocation size or caps is stated anywhere in the issue or in #562. The "4K with 50K free, 34K largest" figures cannot be matched to a call site.

Allocation and sizing path on pinned main `4d79c290`:
- [observed] `psram_allocator<T>::allocate` (`include/interfaces.h:94-99`): `heap_caps_malloc(SPIRAM|8BIT)`, then on failure `heap_caps_malloc(INTERNAL|8BIT)`, then `throw std::bad_alloc`. It has a fallback and the capability sets are right for a byte-addressed buffer. It makes no total-free or largest-block decision itself.
- [observed] The only project-side admission decision is `SystemContainer::SetupBufferManagers` (`src/systemcontainer.cpp:213-241`): free memory = `ESP.getFreePsram()` if `USE_PSRAM`, else `ESP.getFreeHeap()`; `memtouse = free - RESERVE_MEMORY` (saturating); `cBuffers = memtouse / bytes-per-set`; throw if `< MIN_BUFFERS` (3); clamp to `MAX_BUFFERS` (24 without PSRAM, `include/globals.h:425-430`). `RESERVE_MEMORY` is 150000 without PSRAM (`globals.h:547-552`).
- [observed] The issue-era code was `ESP.getFreeHeap() - RESERVE_MEMORY` with unsigned arithmetic (`960daf50:include/systemcontainer.h:163`). The wrap-around was fixed in `b13a22ec` (2026-07-12, "Saturate at zero" comment, `systemcontainer.cpp:219-224`). [inferred] Before that fix a board with less free memory than the reserve got `MAX_BUFFERS` buffers regardless; whether any shipped config hit that is not known.
- [observed] `-DUSE_PSRAM=0` appears nowhere, so the `#ifdef USE_PSRAM` at `globals.h:548` versus `#if USE_PSRAM` elsewhere does not bite today (grep over include, src, platformio.ini). Dropped as a lead.
- [observed] Main already prints `LargestBlk` and `PSRAM Free` (`ESP.getMaxAllocHeap()`, e.g. `src/taskmgr.cpp:156`, `src/main.cpp:861`). It does not register an allocation-failure hook and prints nothing per capability.
- [observed] Pinned toolchain: platform `55.03.37` -> arduino-esp32 3.3.7 and ESP-IDF v5.5.2 (`platform.json` lines 32-61 of the release zip). The 2023 report ran on an older stack (Arduino 2.x, IDF 4.4, same `Esp.cpp` query).

ESP-IDF semantics (all quoted from source; ESP-IDF v5.5.2 unless stated):
- [observed] `EspClass::getFreeHeap()` is `heap_caps_get_free_size(MALLOC_CAP_INTERNAL)` and `getMaxAllocHeap()` is `heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL)` (arduino-esp32 3.3.7 `cores/esp32/Esp.cpp:163-173`; same in 2.0.14).
- [observed] Both are computed by summing, respectively taking the max over every heap whose capabilities include the request (`components/heap/heap_caps.c:277-286` and `:367-389`).
- [observed] On the original ESP32 the IRAM region has capabilities `INTERNAL|EXEC|32BIT` and no `8BIT` (`components/heap/port/esp32/memory_layout.c:16-18,54`; docs `mem_alloc.rst:27,36,69-72`: "At startup, the IRAM heap contains all instruction memory that is not used by the app executable code"; `heap_caps_get_free_size(MALLOC_CAP_8BIT)` is the free size of all DRAM heaps). Same row in IDF 4.4.5 (`memory_layout.c:25-27`).
- [observed] A heap can serve a request only if it has all requested caps (`components/heap/heap_caps_base.c:147`). So an `INTERNAL|8BIT` allocation can never come from IRAM, while `getFreeHeap()` and `getMaxAllocHeap()` count it.
- [observed] Plain `malloc` is `heap_caps_malloc_default`, caps `DEFAULT|INTERNAL` (`heap_caps.c:107-111`); IRAM has no `DEFAULT` cap, so it is not used. With PSRAM enabled and `heap_caps_malloc_extmem_enable(limit)` (project sets 96, `src/main.cpp:387`), requests over `limit` go to SPIRAM first and then fall back to default (`heap_caps.c:113-131`).
- [observed] `heap_caps_register_failed_alloc_callback` exists and reports size, caps and function name of every failed allocation (`heap_caps.c:49-53,62-70`; `esp_heap_caps.h:52-66`).
- [not verified] TLSF size-class rounding and per-block overhead (multi_heap). A block of at least the requested size is therefore necessary, not proven sufficient. Not read from `multi_heap.c`; the model below does not claim it.

## Reproduction

- [tested, model only; not a reproduction] Host logic test of the new helper `FreeInternalBytes8Bit()` (real project header from the fork branch) against a model of the documented capability rules. Run: `docs/external/nightdriver/565/harness/run.sh` (needs a checkout of the fork branch, default `~/workspace/external/nd565-fix`); output in `harness/results.txt`:
  - Case 1, illustrative layout (160000 B of 8-bit DRAM, 90000 B of IRAM, 9000 B per buffer set): the old query (`getFreeHeap` semantics, 250000) admits 11 buffer sets and leaves 61000 B of 8-bit memory, below the 150000 B reserve; the new query (160000) refuses cleanly.
  - Case 2, 20000 deterministic layouts: invariant "after admitting, 8-bit free >= RESERVE" is violated 14018 times by the old query and 0 times by the new one. (Violations scale with IRAM free; the sizes are synthetic. This says the quantity is wrong, not how often it matters on a real board.)
  - Case 3, the issue's literal numbers as a model shape: INTERNAL free 50000, largest 34000, but the largest block is IRAM and DRAM has only fragments up to 3000. A 4096 B `INTERNAL|8BIT` allocation fails, a 4096 B `32BIT` one succeeds. [hypothesized] This is one way the issue's numbers could be true. It is not shown to be what happened.
- [observed] In the #562 log the INTERNAL largest block fell by exactly 4096 (90100 at the Audio launch line, 86004 at the Remote launch line). Between those two lines the Audio Sampler task is created with `AUDIO_STACK_SIZE` 4096 (`taskmgr.h:55,315` at `960daf50`). [inferred] The largest INTERNAL block was therefore the one a task stack was carved from; task stacks need byte access, so it was DRAM, not IRAM (the port code that allocates stacks was not read). That weakens the IRAM hypothesis for the #562 board. [inferred] So the M5StickC Plus numbers in #562 are not explained by IRAM counting; the later "47K free, 31K largest, web server fails" is real shortage for a larger allocation, not a 4K one.
- [BLOCKED] True fragmentation behaviour cannot be reproduced on this host: it depends on the TLSF multi_heap, the real task and WiFi allocation order, and the chip's actual region sizes. No claim of "reproduced" is made for the original failure.

## Root Cause

Not proven. What is established (observed, from source): the project sizes its LED buffer budget from a quantity (`INTERNAL`, aggregate) that is not the quantity its allocator consumes (`INTERNAL|8BIT`, per block). What is not established: that this mismatch caused any failure in #562 or in Dave's report, and which allocation failed. The #562 thread points to a different, ordinary cause for the rebooting board: a fixed 150 kB reserve consumed at runtime by WiFi, task stacks and the web server on a board with ~180 kB free at boot (observed numbers above), plus a framework update (rbergen: ~30K). That is a capacity problem, not an allocator-decision defect.

## Fix

Branch `fix/565-buffer-budget-8bit` on `aien-dev/NightDriverStrip` (fork), commit `8e3d3515c36bebea464467b7834c8ca95afe8925`, based on `4d79c290`. Two files: new `include/heapbudget.h` (`FreeInternalBytes8Bit()`, one `heap_caps_get_free_size(INTERNAL|8BIT)` call) and a 3-line change in `SystemContainer::SetupBufferManagers` for the non-PSRAM branch. PSRAM builds unchanged.
- [tested, model-level; not fixed on a device] The invariant above holds for the new query in the model.
- [not tested] Not compiled for an ESP32 target (no PlatformIO toolchain here); not run on a device. Behaviour change: on classic ESP32 without PSRAM the buffer count can drop by about (IRAM free / bytes per set), so a few boards may need MAX_BUFFERS or MIN_BUFFERS review. That cost is the point: the old count was too high by that amount.
- It does not claim to fix #565. See `upstream-pr-draft.md` for the wording.

## Regression

`harness/run.sh` (above), 3 cases. Red/green: the old query violates the invariant 14018 times and the new helper 0 times in the same run; negative control [tested]: the same test against a copy of the helper that queries `INTERNAL` only prints `RESULT FAIL (3)` (both recorded in `harness/results.txt`). Limits: the arithmetic from `systemcontainer.cpp` is copied into the test, not compiled from source; the heap is a model.

## Instrumentation proposal (device run; nothing applied)

The decisive missing datum is the failing allocation's size and caps. IDF already provides it (`heap_caps_register_failed_alloc_callback`). Proposed minimal diagnostic, in project style, to be tried on a device first (untested):
1. At boot, `heap_caps_register_failed_alloc_callback(cb)`. The callback only stores `size`, `caps`, `function_name`, a failure counter and a snapshot of `heap_caps_get_free_size` and `heap_caps_get_largest_free_block` for `INTERNAL`, `INTERNAL|8BIT`, `DMA`, `SPIRAM|8BIT` into a static struct. No printing, no allocation, no locks in the callback (it runs inside the allocator; `esp_heap_caps.h:68-77` warns about the neighbouring hook for the same reason).
2. Print that struct from the existing periodic status line (`main.cpp:861` area) next to `LargestBlk`, plus `heap_caps_get_minimum_free_size(INTERNAL|8BIT)` and `uxTaskGetStackHighWaterMark` per task where `taskmgr.cpp:156` already logs thread launches.
3. Replace `ESP.getMaxAllocHeap()` in those lines (or add beside it) with the `INTERNAL|8BIT` largest block so the logged number is the one an 8-bit allocation can use.
4. `CONFIG_HEAP_ABORT_WHEN_ALLOCATION_FAILS` (`heap_caps.c:29,55`) is an sdkconfig option. Not checked whether pioarduino 55.03.37 can set it for the Arduino libraries; the callback does not need it.

## Scope

- Only `SetupBufferManagers`' non-PSRAM free-memory source changes. No protocol, architecture, allocator-interface or configuration change; no change to PSRAM routing or `RESERVE_MEMORY`.
- Not examined: `String` fragmentation (robertlipe's hypothesis), WiFi stack growth, effect-internal allocations. They remain candidates for the original failure.
- Device run needed to close the issue: an M5StickC Plus (classic ESP32, no PSRAM), env `m5demo` or `fanset`-equivalent from current main, with the instrumentation above, WiFi and web server on, cycling effects until `bad_alloc`; capture the failing size and caps and the per-cap largest blocks at that instant. A second run on a PSRAM board (to see `extmem` fallback behaviour) is optional.

## INTERPLANE Lesson

Admission decisions must use the capability-qualified resource the consumer will actually draw from, not an aggregate that merely sounds like it.
