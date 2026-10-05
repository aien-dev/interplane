"""Session state limits and close_trace (spec/CORE.md, "Session state limits").

The same scenarios, code strings and message strings are tested in
rust/crates/interplane-crossveil/tests/session_limits.rs.
"""

import pytest

from interplane import lenshift
from interplane.core import Decision, ErrorCode, InputRecord, Limits
from interplane.crossveil import (
    ContinuationRefused,
    MockRuntime,
    Pipeline,
    mock_mapping_table,
)

NOW = "2026-01-01T00:00:00Z"


def pipe(**limits):
    return Pipeline(lenshift, mock_mapping_table(), MockRuntime(), Limits(**limits))


def env(trace, message_id, request_id):
    return {
        "interplane_version": "0.1",
        "message_id": message_id,
        "trace_id": trace,
        "parent_id": None,
        "timestamp": NOW,
        "source": {"kind": "model", "id": "m"},
        "destination": {"kind": "runtime", "id": "mock"},
        "payload": {
            "kind": "tool_request",
            "request_id": request_id,
            "tool": {"namespace": None, "name": "read_file"},
            "arguments": {"path": "/tmp/a"},
            "provenance": {"dialect": "openai", "parser_version": "1.0.0"},
        },
    }


def admit(p, trace, message_id, request_id):
    """``(status, code, message)`` of one admitted envelope."""
    res, _ = p.admit_envelope(env(trace, message_id, request_id))
    if res.error is None:
        return res.status, None, None
    return res.status, res.error.code, res.error.message


def input_rec(input_id, trace):
    return InputRecord.from_dict(
        {
            "input_id": input_id,
            "content_kind": "user_request",
            "trust": "user_supplied",
            "source": {"kind": "operator", "id": "op"},
            "origin": "user:prompt",
            "content_digest": "sha256:" + "a" * 64,
            "trace_id": trace,
            "parent_id": None,
            "derived_from": [],
        }
    )


def continuation(request_id, approval_id):
    return Decision.from_dict(
        {
            "kind": "decision",
            "request_id": request_id,
            "decision": "authorized",
            "capability": "delete_file",
            "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
            "approval": {"approval_id": approval_id},
        }
    )


OK = ("ok", None, None)
LIMIT = "session_limit_exceeded"


def test_defaults():
    lim = Limits()
    assert (lim.max_traces, lim.max_messages_per_trace) == (1024, 4096)
    assert (lim.max_requests_per_trace, lim.max_inputs_per_trace) == (4096, 4096)
    assert Limits.from_dict({"max_traces": 7}).max_traces == 7
    assert ErrorCode.SESSION_LIMIT_EXCEEDED == "session_limit_exceeded"
    assert ErrorCode.SESSION_CLOSED == "session_closed"


def test_request_limit_rejects_before_runtime_contact():
    p = pipe(max_requests_per_trace=2)
    assert admit(p, "t", "m1", "r1") == OK
    assert admit(p, "t", "m2", "r2") == OK
    calls = p.runtime.decide_calls
    got = admit(p, "t", "m3", "r3")
    assert got == ("rejected", LIMIT, "session limit exceeded: max_requests_per_trace")
    assert p.runtime.decide_calls == calls
    # the request id was not recorded: the same id is not a duplicate on a later try elsewhere
    assert not p.ledger.has_request_id("t", "r3")
    # a duplicate is still reported as a duplicate, ahead of the limit
    assert admit(p, "t", "m4", "r1")[1] == "duplicate_request_id"


def test_replay_is_still_reported_at_the_message_limit():
    p = pipe(max_messages_per_trace=2)
    assert admit(p, "t", "m1", "r1") == OK
    assert admit(p, "t", "m2", "r2") == OK
    got = admit(p, "t", "m1", "r9")
    assert got == ("rejected", "replayed_message", "replayed message_id: m1")


def test_message_limit():
    p = pipe(max_messages_per_trace=2)
    assert admit(p, "t", "m1", "r1") == OK
    assert admit(p, "t", "m2", "r2") == OK
    got = admit(p, "t", "m3", "r3")
    assert got == ("rejected", LIMIT, "session limit exceeded: max_messages_per_trace")
    assert not p.ledger.has_message("t", "m3")
    assert admit(p, "u", "m1", "r1") == OK  # other traces are not affected


def test_trace_limit_and_close_frees_a_slot():
    p = pipe(max_traces=2)
    assert admit(p, "a", "m1", "r1") == OK
    assert admit(p, "b", "m1", "r1") == OK
    got = admit(p, "c", "m1", "r1")
    assert got == ("rejected", LIMIT, "session limit exceeded: max_traces")
    assert not p.ledger.holds("c") and not [e for e in p.events if e.extra["trace_id"] == "c"]
    assert admit(p, "a", "m2", "r2") == OK  # a held trace keeps working
    with pytest.raises(ValueError, match="^session limit exceeded: max_traces$"):
        p.register_input(input_rec("i1", "c"))  # inputs cannot open a further trace either
    assert p.inputs("c") == []
    p.close_trace("a")
    assert admit(p, "c", "m1", "r1") == OK


def test_input_limit_records_nothing():
    p = pipe(max_inputs_per_trace=1)
    p.register_input(input_rec("i1", "t"))
    with pytest.raises(ValueError, match="^session limit exceeded: max_inputs_per_trace$"):
        p.register_input(input_rec("i2", "t"))
    assert [r["input_id"] for r in p.inputs("t")] == ["i1"]
    with pytest.raises(ValueError, match="^duplicate input_id: i1$"):
        p.register_input(input_rec("i1", "t"))


def test_close_trace():
    p = pipe()
    assert admit(p, "t", "m1", "r1") == OK
    p.register_input(input_rec("i1", "t"))
    p.run_turn(
        "openai",
        "m",
        {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "c1",
                    "type": "function",
                    "function": {"name": "delete_file", "arguments": '{"path": "/tmp/x"}'},
                }
            ],
        },
        "t",
        0,
    )
    pa = p.pending_approval("t", "c1")
    assert pa is not None
    assert admit(p, "u", "m1", "r1") == OK
    n_t = len([e for e in p.events if e.extra["trace_id"] == "t"])
    n_u = len([e for e in p.events if e.extra["trace_id"] == "u"])
    assert n_t > 0 and n_u > 0

    p.close_trace("t")
    p.close_trace("t")  # closing twice is a no-op
    p.close_trace("never-seen")  # so is closing an unknown trace
    assert len(p.events) == n_u and all(e.extra["trace_id"] == "u" for e in p.events)
    assert p.inputs("t") == [] and p.pending_approval("t", "c1") is None

    closed = ("rejected", "session_closed", "session closed: t")
    assert admit(p, "t", "m9", "r9") == closed
    assert admit(p, "t", "m1", "r1") == closed  # a former replay is refused as closed too
    with pytest.raises(ValueError, match="^session closed: t$"):
        p.register_input(input_rec("i2", "t"))
    d = continuation("c1", pa.approval_id)
    with pytest.raises(ContinuationRefused) as e1:
        p.continue_approval("t", "c1", d, pa.request_digest, NOW)
    assert e1.value.reason == "session_closed"
    with pytest.raises(ContinuationRefused) as e2:
        p.cancel_approval("t", "c1")
    assert e2.value.reason == "session_closed"
    assert p.inputs("t") == [] and len(p.events) == n_u  # a refusal leaves no state behind
    assert admit(p, "u", "m2", "r2") == OK  # other traces keep working
    # a model turn on the closed trace is refused per call and recorded nowhere
    out = p.run_turn(
        "openai",
        "m",
        {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "c2",
                    "type": "function",
                    "function": {"name": "read_file", "arguments": '{"path": "/tmp/x"}'},
                }
            ],
        },
        "t",
        1,
    )
    assert out.results[0].error.code == "session_closed"
    assert out.results[0].error.message == "session closed: t"
    assert not p.ledger.holds("t") and p.inputs("t") == []
    assert all(e.extra["trace_id"] != "t" for e in p.events)


def test_refused_call_uses_its_request_id():
    """A malformed call's result cites its request id, so no later call in the trace may reuse it
    (same in both SDKs)."""

    def call(cid, args):
        return {"id": cid, "type": "function", "function": {"name": "read_file", "arguments": args}}

    def turn(*calls):
        return {"role": "assistant", "content": None, "tool_calls": list(calls)}

    p = pipe()
    out = p.run_turn("openai", "m", turn(call("c8", "{bad"), call("c8", '{"path": "/tmp/a"}')), "t", 0)
    assert [r.error.code for r in out.results] == ["malformed_tool_call", "duplicate_request_id"]
    out = p.run_turn("openai", "m", turn(call("c8", '{"path": "/tmp/a"}')), "t", 1)
    assert out.results[0].error.code == "duplicate_request_id"
    assert out.results[0].error.message == "duplicate request_id: c8"


# -- closed-trace retirement (spec/CORE.md, "Closing a trace") ------------------------------------
# Same scenarios and literals in rust/crates/interplane-crossveil/tests/session_limits.rs.

CYCLES = 100_000
# Fresh ids f0..f39 a 32-bit filter holding r0..r7 refuses (sha256 positions, CORE.md).
RETIRED_FILTER_PARITY = [6, 18, 19, 28, 30, 31, 37, 38]


def open_and_close(p, trace):
    assert admit(p, trace, "m1", "r1") == OK
    p.close_trace(trace)


def closed(trace):
    return ("rejected", "session_closed", f"session closed: {trace}")


def test_retirement_defaults():
    lim = Limits()
    assert (lim.max_closed_traces, lim.retired_filter_bits) == (4096, 8_388_608)
    assert Limits.from_dict({"max_closed_traces": 3}).max_closed_traces == 3


def test_closing_an_unknown_trace_retains_nothing():
    p = pipe()
    for i in range(10_000):
        p.close_trace(f"never-seen-{i}")
    assert p.closed_trace_state() == (0, 0)
    # the trace was never closed, so its first message is a fresh trace
    assert admit(p, "never-seen-0", "m1", "r1") == OK
    p.close_trace("never-seen-0")
    assert p.closed_trace_state() == (1, 0)
    p.close_trace("never-seen-0")  # closing a closed trace is a no-op too
    assert p.closed_trace_state() == (1, 0)


def test_retired_trace_stays_refused():
    p = pipe(max_closed_traces=2)
    for t in ("a", "b", "c"):
        open_and_close(p, t)
    # "a" left the window of the last 2 closed ids and was retired into the filter
    assert p.closed_trace_state() == (2, 1_048_576)
    assert admit(p, "a", "m1", "r1") == closed("a")  # the replay
    assert admit(p, "a", "m2", "r2") == closed("a")  # and any new message
    with pytest.raises(ValueError, match="^session closed: a$"):
        p.register_input(input_rec("i1", "a"))
    p.close_trace("a")  # closing a retired trace is a no-op
    assert p.closed_trace_state() == (2, 1_048_576)
    assert not p.ledger.holds("a") and p.inputs("a") == []
    assert all(e.extra["trace_id"] != "a" for e in p.events)
    assert admit(p, "d", "m1", "r1") == OK  # a fresh trace is admitted


def test_retired_trace_refuses_continuations():
    p = pipe(max_closed_traces=1)
    p.run_turn(
        "openai",
        "m",
        {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "c1",
                    "type": "function",
                    "function": {"name": "delete_file", "arguments": '{"path": "/tmp/x"}'},
                }
            ],
        },
        "t",
        0,
    )
    pa = p.pending_approval("t", "c1")
    assert pa is not None
    p.close_trace("t")
    open_and_close(p, "u")  # pushes "t" out of the window
    assert p.closed_trace_state() == (1, 1_048_576)
    with pytest.raises(ContinuationRefused) as e1:
        p.continue_approval("t", "c1", continuation("c1", pa.approval_id), pa.request_digest, NOW)
    assert e1.value.reason == "session_closed"
    with pytest.raises(ContinuationRefused) as e2:
        p.cancel_approval("t", "c1")
    assert e2.value.reason == "session_closed"


def test_zero_filter_bits_refuses_every_new_trace_after_a_retirement():
    p = pipe(max_closed_traces=0, retired_filter_bits=0)
    assert admit(p, "held", "m1", "r1") == OK
    assert admit(p, "fresh-before", "m1", "r1") == OK
    open_and_close(p, "a")
    assert p.closed_trace_state() == (0, 0)
    assert admit(p, "a", "m1", "r1") == closed("a")
    assert admit(p, "never-used", "m1", "r1") == closed("never-used")  # cannot be told apart
    assert admit(p, "held", "m2", "r2") == OK  # held traces keep working


def test_retirement_filter_is_identical_across_sdks():
    """A 32-bit filter makes false refusals frequent; both SDKs refuse exactly the same fresh ids."""
    p = pipe(max_closed_traces=0, retired_filter_bits=32)
    for i in range(8):
        open_and_close(p, f"r{i}")
    assert p.closed_trace_state() == (0, 4)
    assert all(admit(p, f"r{i}", "m2", "r2") == closed(f"r{i}") for i in range(8))
    refused = [i for i in range(40) if admit(p, f"f{i}", "m1", "r1")[1] == "session_closed"]
    assert refused == RETIRED_FILTER_PARITY


def test_open_close_cycles_stay_within_the_bound_and_replays_stay_refused():
    p = pipe()
    for i in range(CYCLES):
        open_and_close(p, f"s{i}")
    lim = p.limits
    assert p.closed_trace_state() == (lim.max_closed_traces, lim.retired_filter_bits // 8)
    assert p.events == [] and not list(p.ledger.traces())
    for i in range(CYCLES):
        assert admit(p, f"s{i}", "m1", "r1") == closed(f"s{i}")
    assert p.closed_trace_state() == (lim.max_closed_traces, lim.retired_filter_bits // 8)


def test_unknown_close_does_not_poison_the_trace():
    """Before the fix both SDKs recorded the id of an unknown trace on close: its first message was
    then refused ``session_closed``."""
    p = pipe()
    p.close_trace("never-seen")
    assert admit(p, "never-seen", "m1", "r1") == OK
