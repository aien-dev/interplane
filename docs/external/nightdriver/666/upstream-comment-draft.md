# DRAFT, NOT SENT. For the owner to post or discard. Do not mention anything outside NightDriverStrip.

Possible comment for issue 666 (the first reporter's problem):

> A note on the number: 31202 is 0x79E2, so the bytes `E2 79` were read where a command id belongs. That is consistent with the receiver having lost frame alignment partway through a PEAKDATA frame: in builds with audio disabled, before commit d73d18db, the PEAKDATA branch of `socketserver.cpp` read only the 24 byte header and left the audio payload in the stream, so the next "header" was payload bytes. d73d18db now reads and discards the declared payload. (Whether those two bytes were the low bits of the first band value can't be confirmed from the report.)
>
> I built the real `socketserver.cpp` on a Linux host (stubbed platform layer, loopback TCP) and fed it audioserver.py-shaped frames: before d73d18db it reports "Unknown command in packet received: 31202" for a frame whose first float ends in those bytes; at d73d18db and on current main every frame is consumed, with audio enabled or disabled. This was not run on a physical ESP32 and does not cover lwIP behaviour.
>
> Two things to check on your side even with a newer build:
> 1. With `ENABLE_AUDIO 0` (as in the posted `LEDSTRIP` config) the firmware accepts and discards the peak data, so the sample cannot drive anything. With `ENABLE_AUDIO 1` the band count in the script has to equal the firmware's `NUM_BANDS`. The script you pasted has `num_bands = 12`; the copy in the repository has 16, which matches the default. A 12-band sender is rejected as "Invalid peak data packet".
> 2. I have not looked at how remote peak data behaves when no microphone is attached.
