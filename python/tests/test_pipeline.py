import pytest

from interplane.core import Decision, ErrorCode, Limits
from interplane.crossveil import (
    MockRuntime,
    Pipeline,
    default_pipeline,
    make_result,
    mock_mapping_table,
)
from interplane import lenshift


def call(name, args, cid="c1"):
    import json

    return {
        "role": "assistant",
        "content": None,
        "tool_calls": [
            {
                "id": cid,
                "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)},
            }
        ],
    }


def run(pipe, name, args, cid="c1", trace="t", turn=0):
    return pipe.run_turn("openai", "m", call(name, args, cid), trace, turn)


class Scripted(MockRuntime):
    def __init__(self, decide=None, execute=None):
        super().__init__()
        self._d, self._e = decide, execute

    def decide(self, req, ctx):
        self.decide_calls += 1
        if self._d is None:
            return super().decide(req, ctx)
        return self._d(req)

    def execute(self, req, decision, ctx):
        self.execute_calls += 1
        if self._e is None:
            return (
                MockRuntime.execute(self, req, decision, ctx)
                if False
                else super().execute(req, decision, ctx)
            )
        return self._e(req)


def pipe_with(rt):
    return Pipeline(lenshift, mock_mapping_table(), rt)


def test_authorized_flow_and_counts():
    p = default_pipeline()
    out = run(p, "read_file", {"path": "/a"})
    rec = out.observed[0].to_dict()
    assert list(rec) == [
        "request_id",
        "stage",
        "decision",
        "status",
        "error_code",
        "decide_invoked",
        "execute_invoked",
        "result_digest",
    ]
    assert rec["stage"] == "SUCCEEDED" and rec["result_digest"].startswith("sha256:")
    assert (p.runtime.decide_calls, p.runtime.execute_calls) == (1, 1)
    res = out.results[0]
    assert (res.provenance["trust"], res.provenance["content_kind"]) == ("unknown", "tool_result")


def test_result_digest_ignores_duration():
    a = run(default_pipeline(), "read_file", {"path": "/a"}).observed[0].result_digest

    class Timed(MockRuntime):
        def execute(self, req, decision, ctx):
            r = super().execute(req, decision, ctx)
            r.provenance["duration_ms"] = 987
            return r

    b = run(pipe_with(Timed()), "read_file", {"path": "/a"}).observed[0].result_digest
    assert a == b


@pytest.mark.parametrize(
    "name,args,status,code,executed",
    [
        ("write_file", {"path": "p", "content": "c"}, "denied", "policy_denied", 0),
        ("send_email", {"to": "a", "body": "b"}, "denied", "policy_denied", 0),
        ("delete_file", {"path": "p"}, "requires_approval", "approval_required", 0),
        ("read_file", {}, "rejected", "invalid_arguments", 0),
        ("read_file", {"path": 5}, "rejected", "invalid_arguments", 0),
        ("fail_tool", {}, "error", "execution_error", 1),
        ("slow_tool", {}, "timed_out", "execution_timeout", 1),
    ],
)
def test_mock_outcomes(name, args, status, code, executed):
    p = default_pipeline()
    out = run(p, name, args)
    assert (out.results[0].status, out.results[0].error.code) == (status, code)
    assert p.runtime.execute_calls == executed


def test_extra_args_tolerated_and_not_found():
    p = default_pipeline()
    assert run(p, "read_file", {"path": "p", "extra": 1}).results[0].status == "ok"
    out = p.run_turn("openai", "m", call("filesystem.stat", {"path": "p"}, "c9"), "t", 1)
    assert (
        out.observed[0].decision == "not_found"
        and out.results[0].error.code == "capability_not_found"
    )


def test_unknown_decision_is_denied_and_never_executes():
    rt = Scripted(
        decide=lambda req: Decision(req.request_id, "maybe", {"runtime": "x", "policy_engine": "y"})
    )
    p = pipe_with(rt)
    out = run(p, "read_file", {"path": "p"})
    assert (
        out.results[0].status == "denied"
        and out.results[0].error.code == ErrorCode.UNKNOWN_DECISION
    )
    assert rt.execute_calls == 0 and out.observed[0].stage == "DENIED"


def test_decide_exception_fails_closed():
    def boom(req):
        raise RuntimeError("down")

    rt = Scripted(decide=boom)
    out = run(pipe_with(rt), "read_file", {"path": "p"})
    assert out.results[0].error.code == ErrorCode.RUNTIME_UNAVAILABLE and rt.execute_calls == 0
    assert out.observed[0].decision == "denied" and out.observed[0].decide_invoked


def test_garbage_or_mismatched_decision_is_denied():
    for bad in (
        None,
        "authorized",
        {"kind": "decision"},
        Decision("someone-else", "authorized", {"runtime": "x", "policy_engine": "y"}),
    ):
        rt = Scripted(decide=lambda req, bad=bad: bad)
        out = run(pipe_with(rt), "read_file", {"path": "p"})
        assert out.results[0].status == "denied" and rt.execute_calls == 0


def test_execute_exception_becomes_execution_error():
    def boom(req):
        raise RuntimeError("x")

    rt = Scripted(execute=boom)
    out = run(pipe_with(rt), "read_file", {"path": "p"})
    assert (out.results[0].status, out.results[0].error.code) == ("error", "execution_error")
    assert out.observed[0].stage == "FAILED"


def test_missing_trust_defaults_to_untrusted():
    rt = Scripted(execute=lambda req: make_result(req.request_id, "ok", runtime="mock", data=1))
    out = run(pipe_with(rt), "read_file", {"path": "p"})
    assert out.results[0].provenance["trust"] == "unknown"


def test_pipeline_never_caches_or_upgrades_decisions():
    p = default_pipeline()
    run(p, "write_file", {"path": "p", "content": "c"}, cid="a")
    out = run(
        p,
        "write_file",
        {"path": "p", "content": "c", "approval_id": "mock-approval-a"},
        cid="b",
        turn=1,
    )
    assert (
        out.results[0].status == "denied"
        and p.runtime.decide_calls == 2
        and p.runtime.execute_calls == 0
    )
    # approval claims in arguments are never read: delete still needs approval on every request
    out = run(p, "delete_file", {"path": "p", "approval_id": "mock-approval-d"}, cid="d", turn=2)
    assert out.results[0].status == "requires_approval"


def test_oversized_limit_and_per_turn_limit():
    p = default_pipeline(Limits(max_argument_bytes=100, max_requests_per_turn=2))
    out = run(p, "read_file", {"path": "x" * 200})
    assert out.results[0].error.code == "oversized_arguments" and p.runtime.decide_calls == 0
    calls = [
        {
            "id": f"k{i}",
            "type": "function",
            "function": {"name": "list_dir", "arguments": '{"path":"p"}'},
        }
        for i in range(4)
    ]
    out = p.run_turn("openai", "m", {"tool_calls": calls}, "t2", 0)
    assert [r.status for r in out.results] == ["ok", "ok", "rejected", "rejected"]
    assert out.results[2].error.code == "malformed_tool_call"


def test_default_limit_33rd_call_rejected():
    p = default_pipeline()
    calls = [
        {"id": f"k{i}", "function": {"name": "list_dir", "arguments": '{"path":"p"}'}}
        for i in range(33)
    ]
    out = p.run_turn("openai", "m", {"tool_calls": calls}, "t", 0)
    assert sum(r.status == "ok" for r in out.results) == 32 and out.results[32].status == "rejected"


def test_non_tool_request_envelope_is_rejected(envelope):
    p = default_pipeline()
    env = envelope()
    env["payload"] = {
        "kind": "decision",
        "request_id": "r-1",
        "decision": "authorized",
        "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
    }
    res, rec = p.admit_envelope(env)
    assert res.status == "rejected" and rec.stage == "REJECTED" and p.runtime.execute_calls == 0


def test_replay_duplicate_and_malformed(envelope):
    p = default_pipeline()
    assert p.admit_envelope(envelope())[1].status == "ok"
    assert p.admit_envelope(envelope())[1].error_code == "replayed_message"
    assert p.admit_envelope(envelope(message_id="m-2"))[1].error_code == "duplicate_request_id"
    bad = envelope(message_id="m-3")
    del bad["payload"]["request_id"]
    rec = p.admit_envelope(bad)[1]
    assert rec.request_id is None and rec.error_code == "malformed_envelope"
    assert p.runtime.decide_calls == 1


def test_unmapped_name_never_reaches_runtime():
    p = default_pipeline()
    out = run(p, "nonexistent.thing", {})
    assert out.results[0].error.code == "unknown_capability" and p.runtime.decide_calls == 0


def test_events_follow_relayline_order():
    p = default_pipeline()
    out = run(p, "read_file", {"path": "p"})
    assert [e.event for e in out.events] == [
        "tool_request",
        "tool_mapped",
        "tool_decision",
        "tool_execution_started",
        "tool_result",
    ]
    out2 = p.run_turn("openai", "m", {"content": "done"}, "t", 1)
    assert [e.event for e in out2.events] == ["continue", "model_delta", "complete"]
    seqs = [e.seq for e in p.events]
    assert seqs == sorted(seqs) == list(range(len(seqs)))


def test_unsupported_dialect_turn():
    out = default_pipeline().run_turn("frobnicate", "m", "x", "t", 0)
    assert out.to_dict()["error_code"] == "unsupported_dialect" and out.observed == []


PAYLOAD_KEYS = ["data", "decision", "error", "kind", "provenance", "request_id", "status"]
PROV_KEYS = ["capability", "content_kind", "duration_ms", "runtime", "trust", "trusted"]


def payload(name, args, **kw):
    p = default_pipeline(**kw)
    return p, run(p, name, args).results[0].to_dict()


def test_canonical_result_shape_ok_and_denied():
    _, ok = payload("append_note", {"path": "p", "text": "hello"})
    assert sorted(ok) == PAYLOAD_KEYS and sorted(ok["provenance"]) == PROV_KEYS
    assert (
        ok["data"] == {"path": "p", "appended": 5}
        and ok["error"] is None
        and ok["decision"] is None
    )
    assert (ok["provenance"]["trust"], ok["provenance"]["trusted"]) == ("trusted_runtime", True)
    _, den = payload("write_file", {"path": "p", "content": "c"})
    assert sorted(den) == PAYLOAD_KEYS
    assert den["error"] == {
        "code": "policy_denied",
        "message": "mock policy: writes are denied",
        "retryable": False,
    }
    assert sorted(den["decision"]) == [
        "approval",
        "authority",
        "capability",
        "constraints",
        "decision",
        "kind",
        "reason",
        "request_id",
        "runtime_state",
    ]
    assert den["decision"]["authority"] == {
        "decision_id": None,
        "policy_engine": "mock.policy",
        "runtime": "mock",
    }
    assert den["decision"]["constraints"] == [] and den["decision"]["runtime_state"] is None
    assert den["data"] is None
    assert (den["provenance"]["runtime"], den["provenance"]["capability"]) == ("mock", "write_file")
    assert den["provenance"]["trust"] is None and den["provenance"]["trusted"] is None
    _, app = payload("delete_file", {"path": "p"})
    assert app["decision"]["approval"] == {
        "approval_id": "mock-approval-c1",
        "scope": "single_action",
        "expires_at": None,
    }
    assert app["decision"]["reason"] is None
    assert app["error"]["message"] == "approval required: mock-approval-c1"


def test_pre_runtime_rejection_provenance_is_null():
    _, rej = payload("nonexistent.thing", {})
    assert rej["provenance"] == {
        "capability": None,
        "content_kind": None,
        "duration_ms": None,
        "runtime": None,
        "trust": None,
        "trusted": None,
    }
    assert rej["error"] == {
        "code": "unknown_capability",
        "message": "no mapping for tool: nonexistent.thing",
        "retryable": False,
    }


@pytest.mark.parametrize(
    "name,args,code,message,retry",
    [
        ("read_file", {}, "invalid_arguments", "missing required argument: path", False),
        ("read_file", {"path": 1}, "invalid_arguments", "argument path must be string", False),
        (
            "filesystem.stat",
            {"path": "p"},
            "capability_not_found",
            "unknown capability: stat_file",
            False,
        ),
        ("slow_tool", {}, "execution_timeout", "mock execution exceeded 1000 ms", True),
        ("fail_tool", {}, "execution_error", "mock execution failure", False),
    ],
)
def test_pinned_messages(name, args, code, message, retry):
    _, res = payload(name, args)
    assert res["error"] == {"code": code, "message": message, "retryable": retry}


def test_pinned_messages_admission(envelope):
    p = default_pipeline(Limits(max_argument_bytes=10))
    big = envelope(message_id="m-big")
    big["payload"]["arguments"] = {"path": "x" * 50}
    assert p.admit_envelope(big)[0].error.message == "arguments exceed 10 bytes"
    p = default_pipeline()
    p.admit_envelope(envelope())
    assert p.admit_envelope(envelope())[0].error.message == "replayed message_id: m-1"
    assert (
        p.admit_envelope(envelope(message_id="m-2"))[0].error.message == "duplicate request_id: r-1"
    )
    assert (
        p.admit_envelope(envelope(interplane_version="3.0"))[0].error.message
        == "unsupported protocol major version: 3"
    )
    bad = envelope(message_id="m-9", timestamp="x")
    assert p.admit_envelope(bad)[0].error.message == "malformed envelope: timestamp"


def test_unknown_decision_and_runtime_unavailable_messages():
    rt = Scripted(
        decide=lambda req: Decision(req.request_id, "maybe", {"runtime": "x", "policy_engine": "y"})
    )
    res = run(pipe_with(rt), "read_file", {"path": "p"}).results[0]
    assert res.error.message == "unknown decision value: maybe" and res.error.retryable is False

    def boom(req):
        raise RuntimeError("x")

    res = run(pipe_with(Scripted(decide=boom)), "read_file", {"path": "p"}).results[0]
    assert res.error.message == "runtime authority raised an error" and res.error.retryable is True


def test_stale_mapping_rejected_before_decide():
    p = default_pipeline(mapping_table="mock-table-stale")
    out = run(p, "read_file", {"path": "p"})
    res = out.results[0]
    assert (res.status, res.error.code) == ("rejected", "stale_capability")
    assert res.error.message == "mapping table catalog digest does not match runtime catalog"
    assert p.runtime.decide_calls == 0 and out.observed[0].decision is None
    # a table pinned to the live digest is accepted
    from interplane.crossveil import mock_catalog

    live = mock_mapping_table()
    live.catalog_digest = mock_catalog().catalog_digest
    assert (
        run(Pipeline(lenshift, live, MockRuntime()), "read_file", {"path": "p"}).results[0].status
        == "ok"
    )


def test_untrusted_results_render_as_data():
    p = default_pipeline()
    out = run(p, "web_fetch", {"url": "https://x"})
    res = out.results[0]
    assert (res.provenance["trust"], res.provenance["content_kind"], res.provenance["trusted"]) == (
        "external_untrusted",
        "web_content",
        False,
    )
    assert "<tool_call>" in out.rendered[0]["content"]
    nxt = p.run_turn("qwen35", "m", "I will not.", "t", 1)
    assert nxt.intents == 0 and nxt.outcome == "no_tool" and p.runtime.decide_calls == 1
    mem = run(p, "recall_memory", {"query": "q"}, cid="c2", turn=2).results[0]
    assert (mem.provenance["trust"], mem.provenance["content_kind"]) == (
        "workspace_untrusted",
        "memory",
    )


@pytest.mark.parametrize(
    "label, want",
    [
        ({"content_kind": "spreadsheet"}, ("unknown", "unknown", None)),
        ({"content_kind": 7, "trust": ["trusted_runtime"]}, ("unknown", "external_untrusted", False)),
        ({"content_kind": {"k": 1}, "trust": "sorta_trusted"}, ("unknown", "external_untrusted", False)),
        ({"trust": None, "trusted": True}, ("tool_result", "unknown", None)),
    ],
)
def test_unrecognized_or_unparseable_labels_never_become_trusted(label, want):
    # An adapter's label the SDK cannot read: unrecognized or non-string content_kind is
    # unknown (as in Rust, pipeline.rs normalize), unrecognized or non-string trust is
    # external_untrusted, absent/null trust is unknown; the adapter's own trusted flag is ignored.
    p = default_pipeline()
    p.runtime.provenance_overrides = {"read_file": label}
    prov = run(p, "read_file", {"path": "/a"}).results[0].provenance
    assert (prov["content_kind"], prov["trust"], prov["trusted"]) == want


def _rec(input_id, trust, trace="t", derived=()):
    return {
        "input_id": input_id,
        "content_kind": "user_request",
        "trust": trust,
        "source": {"kind": "operator", "id": "op"},
        "origin": "user:prompt",
        "content_digest": "sha256:" + "a" * 64,
        "trace_id": trace,
        "parent_id": None,
        "derived_from": list(derived),
    }


def test_exposure_fails_closed_when_ledger_is_empty():
    pipe = default_pipeline()
    run(pipe, "read_file", {"path": "/x"})
    assert pipe.runtime.seen_exposure == [
        {"request_id": "c1", "inputs": [], "floor": "external_untrusted"}
    ]


def test_exposure_floor_is_least_trusted_and_unknown_ranks_external():
    pipe = default_pipeline()
    pipe.register_input(_rec("a", "trusted_runtime"))
    assert pipe.exposure_for("t") == {"inputs": ["a"], "floor": "trusted_runtime"}
    pipe.register_input(_rec("b", "user_supplied"))
    assert pipe.exposure_for("t")["floor"] == "user_supplied"
    pipe.register_input(_rec("c", "not-a-level"))
    assert pipe.exposure_for("t")["floor"] == "external_untrusted"
    pipe.register_input(_rec("d", "trusted_runtime", trace="other"))
    assert pipe.exposure_for("other") == {"inputs": ["d"], "floor": "trusted_runtime"}


def test_register_input_refuses_duplicate_id():
    pipe = default_pipeline()
    pipe.register_input(_rec("a", "user_supplied"))
    with pytest.raises(ValueError):
        pipe.register_input(_rec("a", "trusted_runtime"))
    assert [r["trust"] for r in pipe.inputs("t")] == ["user_supplied"]


def test_input_registration_is_not_reachable_from_an_envelope():
    pipe = default_pipeline()
    env = {
        "interplane_version": "0.1",
        "message_id": "m1",
        "trace_id": "t",
        "parent_id": None,
        "timestamp": "2026-01-01T00:00:00Z",
        "source": {"kind": "model", "id": "m"},
        "destination": {"kind": "runtime", "id": "mock"},
        "payload": {"kind": "input_record", **_rec("x", "trusted_runtime")},
    }
    result, _ = pipe.admit_envelope(env)
    assert result.error.code == ErrorCode.MALFORMED_ENVELOPE
    assert pipe.inputs("t") == []


# ---- host approval continuation (0.3 cut A2) ----

PINNED_DIGEST = "sha256:be3ae35c3a1f72b4a403479b1f03589af2568892b47f58b48397fdcf2d7090a5"


def _continuation(rid, aid, kind="authorized"):
    return Decision.from_dict(
        {
            "kind": "decision",
            "request_id": rid,
            "decision": kind,
            "capability": "delete_file",
            "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
            "approval": {"approval_id": aid},
        }
    )


def _pending_delete(pipe):
    run(pipe, "delete_file", {"path": "/tmp/x"})
    return pipe.pending_approval("t", "c1")


def test_request_digest_is_pinned_and_matches_rust():
    pa = _pending_delete(pipe_with(MockRuntime()))
    assert pa.request_digest == PINNED_DIGEST
    assert pa.approval_id == "mock-approval-c1"


def test_continuation_executes_once_and_never_calls_decide():
    from interplane.crossveil import ContinuationRefused

    rt = MockRuntime()
    pipe = pipe_with(rt)
    pa = _pending_delete(pipe)
    d = _continuation("c1", pa.approval_id)
    res, rec = pipe.continue_approval("t", "c1", d, pa.request_digest, "2026-01-01T00:00:00Z")
    assert res.status == "ok"
    assert (rec.stage, rec.decide_invoked, rec.execute_invoked) == ("SUCCEEDED", False, True)
    with pytest.raises(ContinuationRefused):
        pipe.continue_approval("t", "c1", d, pa.request_digest, "2026-01-01T00:00:00Z")
    assert (rt.decide_calls, rt.execute_calls) == (1, 1)


@pytest.mark.parametrize(
    "expires,now",
    [
        ("2026-06-01T00:00:00Z", "yesterday"),
        ("2026-06-01", "2026-01-01T00:00:00Z"),
        ("2026-06-01T00:00:00Z", "2026-06-01T00:00:00Z"),
        ("2026-06-01T00:00:00Z", "2026-13-01T00:00:00Z"),
    ],
)
def test_unreadable_clock_or_expiry_counts_as_expired(expires, now):
    rt = MockRuntime()
    rt.approval_expires_at = expires
    pipe = pipe_with(rt)
    pa = _pending_delete(pipe)
    d = _continuation("c1", pa.approval_id)
    res, _ = pipe.continue_approval("t", "c1", d, pa.request_digest, now)
    assert res.status == "denied" and res.error.message == "approval expired"


def test_continuation_leaves_the_request_ledger_alone_and_feeds_the_exposure_ledger():
    pipe = pipe_with(MockRuntime())
    pa = _pending_delete(pipe)
    before = len(pipe.inputs("t"))
    d = _continuation("c1", pa.approval_id)
    _, rec = pipe.continue_approval("t", "c1", d, pa.request_digest, "2026-01-01T00:00:00Z")
    assert len(pipe.inputs("t")) == before + 1
    assert pipe.inputs("t")[-1]["content_digest"] == rec.result_digest
    out = run(pipe, "delete_file", {"path": "/tmp/x"}, turn=1)
    assert out.results[0].error.code == ErrorCode.DUPLICATE_REQUEST_ID
