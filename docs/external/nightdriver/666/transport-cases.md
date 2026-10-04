# Generic transport-conformance cases (extracted from the NightDriver 666 investigation)

Semantic descriptions only. No code from the investigated project is reproduced here. Each case
applies to any receiver that parses length-prefixed frames from a TCP byte stream, which includes
INTERPLANE transports in the 0.4 Runner work.

Notation: a frame is `header (fixed size) + body (length given in header)`. "Consumer" is whatever
the receiver hands complete frames to.

## Stream-alignment property (the one that mattered)

**T-ALIGN-1. Every accepted command consumes its whole declared frame.**
For each command the receiver accepts, including commands it intends to ignore or has disabled,
the receiver must read and discard the full declared body before parsing the next header.
Failure signature: the next "header" is taken from the middle of a previous body, so the first
field is a meaningless value (here a 16 bit command id equal to the low bits of a float).
Test: send `[ignored-command frame][valid frame][ignored-command frame][valid frame]`; require the
valid frames to reach the consumer and no unknown-command rejection.

**T-ALIGN-2. A disabled feature must not change framing.**
Run T-ALIGN-1 once with the feature enabled and once disabled. The consumer-visible result may
differ, but the "no rejection, next frame aligned" outcome must be identical.

## Segmentation

**T-SEG-1. Header split at every byte offset.** One frame, two writes, split at each offset in the header, pause between writes.
**T-SEG-2. Body split at every byte offset.** Same, across the body.
**T-SEG-3. Coalescing.** Several frames in one write; a frame plus the first part of the next; remainder in a later write.
**T-SEG-4. Single-byte writes.** A full frame sent one byte at a time.
Expectation for all: exactly the sent frames reach the consumer, in order, byte-identical.

## Connection lifecycle

**T-LIFE-1. Disconnect mid-header, then reconnect with a valid frame.** No state from the aborted connection may leak: only the valid frame is delivered.
**T-LIFE-2. Disconnect mid-body, then reconnect.** Same.
**T-LIFE-3. Reconnect storm.** Repeated connect, one frame, close. Every frame delivered once.
**T-LIFE-4. Stalled sender.** Header or body paused longer than the receiver's read timeout. The receiver drops the connection and discards the partial frame; the next connection is clean. Paused shorter than the timeout: the frame is delivered.
**T-LIFE-5. Non-reading client.** Client never reads responses. Record whether the receiver keeps consuming. (Observation, not a pass/fail contract.)

## Bounds and malformed input

**T-BOUND-1. Maximum declared size accepted, maximum plus one rejected** with the connection closed and the next connection clean.
**T-BOUND-2. Declared length at the integer limit** (all bits set, and a value whose size computation wraps on a 32 bit target) is rejected without allocating or reading that many bytes.
**T-BOUND-3. Zero-length body.** Record behaviour (consumer called or not, response or not). Observation.
**T-BOUND-4. Unknown command.** Rejected, connection closed, next connection clean, and the rejection reports the first bytes it saw so a human can identify foreign traffic.
**T-BOUND-5. Foreign protocol on the port.** An HTTP request line sent to a raw framed port is rejected as an unknown command and the connection closed. The reported value decodes to the first two ASCII bytes. Error messages for this case should say "unknown command" with the raw bytes, so the operator can recognise a misconfigured client.

## Sender-side faults

**T-SEND-1. Sender wrote a prefix and dropped the tail** (the failure `sendall` prevents and plain `send` permits on short writes). A framed stream cannot detect this. Record what the receiver reports; it is expected to be either a corrupted but well-formed frame followed by an unknown-command rejection, or a pure unknown-command rejection. Mitigation belongs to the sender (loop until all bytes are written) or to the protocol (per-frame checksum or sync marker), not to the receiver's parser.
**T-SEND-2. Sender short-write probe.** Blocking send of a small frame under receiver backpressure: verify whether the platform returns a short count. (Observed on Linux with CPython 3.12: never, including under backpressure.)

## Reading a rejection value

When a receiver reports a single unexpected integer, decode it before debugging anything else:
little-endian bytes of the integer, compare to ASCII (foreign protocol) and to the bytes at each
offset of a valid frame (desync). A value that equals bytes from the middle of a valid frame points
at framing; a value that is printable ASCII points at a foreign client.
