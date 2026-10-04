# Memory cases for NightDriverStrip #565 (generic, reusable)

Each row: situation, which quantity decides, and what the host model or source shows. Tags as in REPORT.md.
Semantics cited from ESP-IDF v5.5.2 (`heap_caps.c`, `heap_caps_base.c:147`, `memory_layout.c:49-54`).

| # | Situation | Deciding quantity | Status |
|---|---|---|---|
| 1 | 8-bit buffer on classic ESP32, IRAM heap holds most of the free memory | `INTERNAL|8BIT` largest block, not `INTERNAL` (IRAM has no 8BIT) | model case 3, hypothesized for the real issue |
| 2 | Budget from `getFreeHeap` while allocating `INTERNAL|8BIT` | `INTERNAL|8BIT` free | model cases 1-2, fixed in the fork branch |
| 3 | Plain `malloc` or `new` | `DEFAULT|INTERNAL` (IRAM excluded), or SPIRAM first above the extmem threshold then default (`heap_caps.c:107-131`) | observed in source |
| 4 | DMA buffer | `DMA|INTERNAL|8BIT` (`interfaces.h:156`); DMA free/largest are logged by the web UI (`webserver.cpp:366-367`) | observed, no case run |
| 5 | PSRAM buffer with internal fallback (`psram_allocator`) | `SPIRAM|8BIT` first, then `INTERNAL|8BIT`; a PSRAM-exhausted board silently spends internal memory | observed in source, no case run |
| 6 | Executable memory | `EXEC` forces IRAM and cannot be combined with `8BIT` or `DMA` (`heap_caps_base.c` EXEC branch) | observed in source, not relevant to NightDriver |
| 7 | Total free large, largest block small | one allocation needs a block, a set of N buffers needs N blocks; "block at least the size" is necessary, TLSF sufficiency not verified | not verified |
| 8 | Reserve consumed at runtime (WiFi, stacks, web server) after a one-time budget | a one-time free-memory budget cannot see later consumers | observed in #562 numbers (180K free at boot, 47K later) |
| 9 | Unsigned `free - reserve` wrap | saturate before dividing | observed, fixed upstream in `b13a22ec` |
