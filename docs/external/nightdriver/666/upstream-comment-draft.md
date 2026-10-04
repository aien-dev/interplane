# DRAFT, NOT SENT. For the owner to post or discard. Do not mention anything outside NightDriverStrip.

Possible close-out comment for issue 666:

> Decoding the two reported values: 31202 is 0x79E2, which is the low 16 bits of the first audio band float of a PEAKDATA frame, and 17735 is 0x4547, which is the ASCII "GE" that begins an HTTP `GET` request.
>
> 31202: this is the receiver losing frame alignment in builds with audio disabled. Before commit d73d18db the PEAKDATA branch read only the 24 byte header and left the payload in the stream, so the next "header" was payload bytes. That commit now reads and discards the declared payload. I could not reproduce it on current main: with real `socketserver.cpp` built on a Linux host and fed audioserver.py-shaped frames, every frame is consumed, with audio enabled or disabled. Updating to a build that includes d73d18db should resolve it.
>
> 17735: as noted above, a websocket client cannot talk to the raw frame port 49152; the "GE" is the start of its HTTP upgrade request.
>
> Both can be closed unless someone still sees it on a build newer than d73d18db.
