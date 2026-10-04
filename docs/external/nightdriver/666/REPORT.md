# NightDriverStrip issue 666: "Socket server: Unknown command in packet received: 31202"

External proving-ground lane D1. Upstream project: PlummersSoftwareLLC/NightDriverStrip.
Nothing here has been sent upstream (tag `upstreamed` is not claimed anywhere below).

**Outcome: B, already fixed upstream.** Reporter 1's error is a receiver framing bug (the receiver
did not consume the payload of a command it chose to ignore). It was fixed upstream in commit
`d73d18db` (2025-08-27) and is absent from pinned main `4d79c290`. Reporter 2's error is a
misconfigured client (a websocket aimed at the raw TCP port), not a bug. No code change is
proposed. The remaining value is the harness and the generic cases.

Evidence tags: observed, reproduced, hypothesized, fixed, tested, upstreamed, accepted.

## Observed

Issue thread read in full (GitHub API; `gh issue view` fails here on a Projects-classic GraphQL error). Two reporters, treated separately.

**Reporter 1, jmadotgg (2024-11-09): value 31202, running `samples/audioserver/audioserver.py` against the ledstrip demo.**
- [observed] The posted `LEDSTRIP` config has `ENABLE_AUDIO 0` (issue body). The sender is `audioserver.py`, which sends command 4 (PEAKDATA) with `num_bands` bands after each audio chunk and a 15 ms sleep (`samples/audioserver/audioserver.py:128-142` at `4d79c290`). The reporter's pasted copy has `num_bands = 12`; the repository copy has had `num_bands = 16` since commit `3146af92` (2023-02-24) and is unchanged between `15221f20` and `4d79c290`. So the reporter ran an edited or older copy. Why is unknown.
- [observed] 31202 = 0x79E2, so on a little-endian wire the two bytes are `E2 79`. The receiver reads `command16` as a little-endian u16 from buffer offset 0 (`WORDFromMemory`).
- [observed] In the reporter-era receiver (`src/socketserver.cpp` at `15221f20`, lines 132-169), the PEAKDATA branch with `ENABLE_AUDIO 0` compiles to nothing but `ResetReadBuffer()`. Only the 24-byte header was read from the socket; the declared 48-byte float payload stays in the stream. The next loop iteration reads 24 "header" bytes from that payload, so `command16` is the low 16 bits of `float[0]`. Anything other than 3 or 4 hits "Unknown command" (line 214), which closes the connection, matching "connection is closing and both sides keep retrying".
- [observed] The variant one commit earlier than the fix (`d73d18db~1`) gates on runtime `g_Analyzer.Enabled()` and has the same flaw (line 135).
- [observed] `d73d18db` adds the missing skip with the comment "Audio disabled: consume any declared payload to keep stream in sync, then ignore it". Pinned main has it at `src/socketserver.cpp:511-527`.
- [hypothesized] The exact value 31202 equals the low 16 bits of the reporter's first band value in their first frame. This is consistent with the mechanism but cannot be checked (many floats share those low bits and their audio is gone). The mechanism, not the number, is what is reproduced.
- [observed] Not the cause: the negative `micros` the reporter noticed (`time.perf_counter() - seconds`) is sent as a double and read as u64; it does not affect framing.
- [reproduced] On the main host build with `ENABLE_AUDIO 1`, the reporter's 12-band frames are rejected: the receiver logs "Invalid peak data packet: bands=12 length=48 total=72" and closes the connection. That is not an "Unknown command" and not a desync. At `4d79c290` `platformio.ini` sets `-DNUM_BANDS` to 16 in nine environments, 8, 48 and 64 in one each, and `globals.h` defaults to 16; no environment uses 12. So a 12-band sender cannot be accepted by any audio-enabled build of main. The repository copy of the script (16 bands) matches the common firmware value; harness case `peak16_with_pixels` shows 16-band frames are accepted with audio on.

**Reporter 2, Cyborg-Squirrel (2025-07-05): value 17735, Kotlin client opening a WebSocket to port 49152.**
- [observed] 17735 = 0x4547; little-endian bytes `47 45` = ASCII "GE", the start of `GET /... HTTP/1.1`, the first bytes of a websocket upgrade request.
- [observed] Maintainer rbergen already said port 49152 is a raw TCP port that takes frame data only and that a websocket can only be opened against the web server on port 80, and only to read data from the device.
- [reproduced] Sending an HTTP GET to the real receiver code yields exactly "Unknown command in packet received: 17735" and a closed connection (case `http_get_on_raw_port`, all builds).
- Verdict: Outcome A (receiver correct, client misconfigured). The error text could say more (see Scope).

## Reproduction

What was built: the project's real `src/socketserver.cpp`, extracted unmodified with `git archive` at four SHAs, compiled on Linux (g++ 13, C++20) against stub platform headers (FreeRTOS task base, logging macros, PSRAM allocators, WiFi state, buffer manager, uzlib), driven over loopback TCP by a Rust harness. Stubs live in `harness/stubs/`; they are written for this harness and contain no project code. For the older snapshots, whose task loop lives in `src/network.cpp` (`SocketServerTaskEntry`, line 697 at `15221f20`), the host driver replicates that loop. Byte-reader helpers are stand-ins for the originals on those snapshots.

Snapshots:
| label | SHA | what it is |
|---|---|---|
| s0 | `15221f20` | last socket-server change before the report |
| s1 | `d73d18db~1` | parent of the fix |
| s2 | `d73d18db` | the fix (2025-08-27) |
| main | `4d79c290` | pinned upstream main (2026-08-15) |

Command (needs a NightDriverStrip clone; no project source is stored in this repo):

    ND_REPO=~/workspace/external/NightDriverStrip \
      docs/external/nightdriver/666/harness/run-matrix.sh > results.tsv

Recorded run: `harness/results.tsv`. Summary of the property cases (PASS means the stream stayed aligned and every frame arrived byte-identical):

| case | s0 | s1 | s2 | main |
|---|---|---|---|---|
| peak12_with_pixels (audioserver shape, float[0] low bits 0x79E2) | FAIL: Unknown command 31202 | FAIL: 31202 | PASS | PASS |
| peak16_with_pixels | FAIL: 31202 | FAIL: 31202 | PASS | PASS |
| whole frames, every split offset, coalescing, 1-byte writes | PASS | PASS | PASS | PASS |
| disconnect mid-header / mid-body then reconnect | PASS | PASS | PASS | PASS |
| unknown command then valid | PASS | PASS | PASS | PASS |
| max accepted, max+1 rejected, 0xFFFFFFFF rejected | PASS | PASS | PASS | PASS |
| slow sender inside / past the 3 s read timeout | PASS | PASS | PASS | PASS |
| HTTP GET on raw port gives 17735 | PASS | PASS | PASS | PASS |
| reconnect storm | PASS | PASS | PASS | PASS |

Audio-on builds (`ENABLE_AUDIO=1`) of s2 and main: all property cases PASS.

- [reproduced] Mechanism for reporter 1: the real receiver source at `15221f20` (a reporter-era snapshot; the reporter's exact firmware commit is unknown, so s0 and s1 are the two flawed endpoints we tested, not "their build"), fed `audioserver.py`-shaped frames, prints `Unknown command in packet received: 31202` when `float[0]` carries bits 0x????79E2. The test input was chosen to produce that value, so this reproduces the failure mode and its exact signature, not the reporter's audio.
- [tested] Red then green: the same peak cases fail on s0 and s1 and pass on s2 and main. This is the negative control for the harness.
- [reproduced] Reporter 2's 17735 from an HTTP request line.

Limits, stated precisely: the s1 snapshot gates on runtime `g_Analyzer.Enabled()`; the host stub models it as the compile-time `ENABLE_AUDIO` value. That is a modelling assumption. [hypothesized, untested] A real build with audio compiled in but no microphone could return false at run time and desync the same way, which would also explain reporter 1 if they had flashed audio on. this is the real socket-server source, not a replica of its parsing, but it runs on Linux loopback TCP with `SO_RCVTIMEO`, not lwIP on an ESP32. Nothing here was run on a physical ESP32 and nothing is claimed about lwIP buffering, task scheduling, or PSRAM behaviour. A device test would need an ESP32 board flashed with the ledstrip demo from a pre-`d73d18db` checkout (and one from main), plus `audioserver.py` pointed at it with a serial monitor to read the log.

## Root Cause

[reproduced] for reporter 1, on the reporter-era source: a command that the firmware decides to ignore (PEAKDATA with audio disabled) was not drained from the TCP stream. The receiver parsed payload bytes as the next header. [observed] Fixed by `d73d18db`.

Reporter 2: no receiver defect. [observed] misdirected client.

## Fix

[fixed] upstream by `d73d18db` (not by us): when audio is disabled, read `STANDARD_DATA_HEADER_SIZE + length32` bytes and discard them before resetting the buffer. [tested] on main by the harness above (both audio settings). Upstream main also validates the length with `CheckedStandardPacketSize` before reading (`src/socketserver.cpp:511-527`).

No fork branch and no pull request: there is no remaining defect on main to fix. The 12 vs `NUM_BANDS` mismatch is in the reporter's pasted script, not in the repository sample (see Observed).

## Regression

[tested] Cases `peak12_with_pixels` and `peak16_with_pixels` fail on the pre-fix snapshots and pass on the fix and on main. Upstream has no automated test environment for the firmware: `platformio.ini` has no test environment, and the only tests in the tree are Python tooling tests (`tools/test_build_tools.py`, `tools/test_topology.py`). So no in-repo test is proposed; the harness stays on our side.

## Scope (not changed)

- No upstream pull request, issue comment or reaction. A suggested close-out comment is in `upstream-comment-draft.md` for the owner to post or discard.
- Nothing about the wire format, audioserver.py, or the socket server was modified. No fork branch or sample change is proposed: the repository sample already uses 16 bands, which matches the firmware default; the 12 in the report came from the reporter's copy.
- [hypothesized, not run] On a 32-bit target the older size computation `STANDARD_DATA_HEADER_SIZE + length32 * LED_DATA_SIZE` can wrap (for example `length32 = 0x55555556`), so a hostile or corrupt header could be accepted with a tiny length. The host harness is 64-bit and cannot show this. Main uses an overflow-checked helper, so it looks handled; unverified on a 32-bit build.
- [observed] Older firmware (s0) sleeps 100 ms after every frame (`delay(100)` at `15221f20` line 247), so it consumes about 10 frames per second; later snapshots sleep 1 ms. `audioserver.py` reads 512 samples at 44.1 kHz (about 11.6 ms) and sleeps 15 ms per loop, so it sends very roughly 35 to 40 frames per second [hypothesized by arithmetic, not measured]; against s0 the sender would outpace the receiver and TCP backpressure would absorb it. Not a framing issue.
- [observed, INFO cases] zero-LED pixel frame is passed to the consumer with length 24. A client that never reads the 72-byte responses is still served on the host (about 1.3k of 4k frames consumed in 1.5 s, limited by the 1 ms per-frame sleep, no stall seen). Sender prefix-then-drop faults (`sender_short_write`) surface as one corrupted-but-well-formed packet then an Unknown command or garbage value, on every build; a framed stream cannot detect that.
- Sender-side Outcome C (Python `send()` short write): [observed] on Linux, CPython 3.12.3, 1342 blocking `send()` calls of a 72-byte frame under receiver backpressure returned 72 every time (scratch script, not kept in the repo). Not supported as a cause of reporter 1; it is not excluded on other platforms.
- Optional upstream polish (not proposed): the "Unknown command" message could also print the first two raw bytes as hex, and the docs could say port 49152 is raw TCP only. Either would have shortened both reporters' debugging.

## INTERPLANE Lesson

- [observed] Decode a single rejected value before anything else: little-endian bytes, compare with ASCII and with every offset of a valid frame. Both reporters were solved by that arithmetic (0x4547 is "GE"; 0x79E2 is payload bytes).
- [observed] The dangerous bug class is not parsing, it is a disabled or ignored path that changes how many bytes get consumed. A transport conformance suite should run every command in enabled and disabled form and require the same stream alignment (cases T-ALIGN-1, T-ALIGN-2 in `transport-cases.md`).
- [tested] Segmentation, coalescing, mid-frame disconnect, timeout and oversize cases all passed on the real receiver even on the buggy snapshots; the one failure was caught only by interleaving an ignored command with valid ones. Interleaved-sequence cases belong in the Runner conformance set next to the split/coalesce cases.
- Error messages for rejected frames should include the raw first bytes. That alone identifies foreign traffic.
- Method: compiling a project's real source unmodified against a small stub platform layer gave reproduction evidence without hardware, and the pre-fix/post-fix commit pair gave the negative control for free.

Generic case list: `transport-cases.md`. Harness: `harness/` (`host/` C++ driver and build script, `stubs/` platform stubs, `torture/` Rust test driver, `run-matrix.sh`, `results.tsv`).
