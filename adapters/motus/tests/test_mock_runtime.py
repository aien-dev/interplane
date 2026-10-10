"""Lifecycle cases against the Core mock runtime (the normative mock of CORE.md)."""

import json

from interplane.crossveil import MockRuntime

from conftest import mock_approver
from interplane_adapter_motus.verify import verify_bundle

J = json.dumps


def test_denied_never_invokes_the_tool(make_mock):
    rt, gate = make_mock()
    out = gate.call("write_file", J({"path": "a", "content": "b"}))
    assert out.record["decision"] == "denied" and not out.executed and rt.execute_calls == 0


def test_held_for_approval_never_invokes_the_tool(make_mock):
    rt, gate = make_mock()
    out = gate.call("delete_file", J({"path": "a"}))
    assert out.outcome == "approval_pending" and not out.executed and rt.execute_calls == 0
    assert out.record["approval_id"].startswith("mock-approval-")


def test_valid_continuation_runs_once_and_second_is_refused(make_mock):
    rt, gate = make_mock()
    rid = gate.call("delete_file", J({"path": "a"})).record["request_id"]
    first = gate.approve(rid, mock_approver())
    assert first.executed and rt.execute_calls == 1
    second = gate.approve(rid, mock_approver())
    assert not second.executed and rt.execute_calls == 1
    # the Pipeline itself also refuses, whatever the adapter does
    from interplane.crossveil import ContinuationRefused
    import pytest
    with pytest.raises(ContinuationRefused):
        gate.pipeline.continue_approval(gate.trace_id, rid, mock_approver()(type("P", (), {
            "request_id": rid, "approval_id": f"mock-approval-{rid}",
            "capability_request": type("C", (), {"capability": "delete_file"})()})()),
            "sha256:" + "0" * 64, "2026-01-01T00:00:00Z")
    assert verify_bundle(gate.evidence()) == []


def test_wrong_approval_id_denies_the_request_for_good(make_mock):
    rt, gate = make_mock()
    rid = gate.call("delete_file", J({"path": "a"})).record["request_id"]

    def wrong(pending):
        d = mock_approver()(pending)
        d.approval["approval_id"] = "someone-elses"
        return d

    out = gate.approve(rid, wrong)
    assert not out.executed
    assert not gate.approve(rid, mock_approver()).executed
    assert rt.execute_calls == 0


def test_declined_decision_never_executes(make_mock):
    rt, gate = make_mock()
    rid = gate.call("delete_file", J({"path": "a"})).record["request_id"]
    assert not gate.approve(rid, mock_approver("denied")).executed
    assert rt.execute_calls == 0


def test_approver_that_fails_or_answers_nonsense_executes_nothing(make_mock):
    rt, gate = make_mock()
    rid = gate.call("delete_file", J({"path": "a"})).record["request_id"]

    def boom(pending):
        raise RuntimeError("no")

    assert gate.approve(rid, boom).record["error_code"] == "approver_failed"
    assert gate.approve(rid, lambda p: "authorized").record["error_code"] == "approver_failed"
    assert rt.execute_calls == 0
    # the failed attempts did not spend the pending entry
    assert gate.approve(rid, mock_approver()).executed


def test_stale_mapping_table_refuses_before_the_runtime(make_mock):
    rt, gate = make_mock(table="mock-table-stale")
    out = gate.call("read_file", J({"path": "a"}))
    assert out.record["error_code"] == "stale_capability" and not out.executed
    assert rt.decide_calls == 0 and rt.execute_calls == 0


def test_catalog_drift_while_waiting_refuses_the_continuation(make_mock):
    from interplane.core import CapabilityDescriptor

    class Drifting(MockRuntime):
        drift = False

        def catalog(self):
            cat = super().catalog()
            if self.drift:
                cat.capabilities.append(CapabilityDescriptor(
                    name="new_tool", description="x", parameters={"type": "object", "properties": {}}))
                cat.catalog_digest = cat.computed_digest()
            return cat

    rt = Drifting()
    _, gate = make_mock(runtime=rt)
    rid = gate.call("delete_file", J({"path": "a"})).record["request_id"]
    rt.drift = True
    out = gate.approve(rid, mock_approver())
    assert not out.executed and out.record["error_code"] == "stale_capability"
    assert rt.execute_calls == 0


def test_duplicate_request_id_replay_fails_closed(make_mock):
    rt, gate = make_mock()
    first = gate.call("read_file", J({"path": "a"}), call_id="dup-1")
    again = gate.call("read_file", J({"path": "a"}), call_id="dup-1")
    assert first.executed and not again.executed
    assert again.record["error_code"] == "duplicate_request_id"
    assert again.record["capability_request_digest"] is None  # the first call's digest is not borrowed
    assert rt.execute_calls == 1 and rt.decide_calls == 1


def test_runtime_not_found_and_invalid_are_preserved(make_mock):
    rt, gate = make_mock()
    out = gate.call("read_file", J({}))
    assert out.record["decision"] == "invalid" and not out.executed


def test_injection_text_from_a_tool_cannot_authorize_an_effect(make_mock):
    rt, gate = make_mock()
    got = gate.call("web_fetch", J({"url": "http://x"}))
    assert got.executed and "send_email" in got.text  # injected text reaches the model as data only
    mail = gate.call("send_email", J({"to": "a@b.c", "body": "x"}))
    assert mail.record["decision"] == "denied" and not mail.executed
    note = gate.call("append_note", J({"path": "n", "text": "t"}))
    assert note.outcome == "approval_pending" and not note.executed  # untrusted input armed the gate
    assert rt.execute_calls == 1


def test_timeout_from_the_runtime_is_not_retried(make_mock):
    rt, gate = make_mock()
    out = gate.call("slow_tool", "{}")
    assert out.outcome == "timeout" and rt.execute_calls == 1
    assert gate.evidence()["records"][-1]["retried"] is False


def test_failed_execution_is_reported_as_failed_once(make_mock):
    rt, gate = make_mock()
    out = gate.call("fail_tool", "{}")
    assert out.outcome == "failed" and rt.execute_calls == 1


def test_runtime_that_raises_on_decide_is_unavailable(make_mock):
    class Down(MockRuntime):
        def decide(self, req, ctx):
            raise OSError("down")

    rt = Down()
    _, gate = make_mock(runtime=rt)
    out = gate.call("read_file", J({"path": "a"}))
    assert out.outcome == "unavailable" and rt.execute_calls == 0


def test_runtime_that_raises_on_execute_is_uncertain_and_not_retried(make_mock):
    class Breaks(MockRuntime):
        def execute(self, req, decision, ctx):
            self.execute_calls += 1
            raise OSError("lost")

    rt = Breaks()
    _, gate = make_mock(runtime=rt)
    out = gate.call("read_file", J({"path": "a"}))
    assert out.outcome == "uncertain" and rt.execute_calls == 1


def test_a_decision_for_another_request_is_never_applied(make_mock):
    class Mixed(MockRuntime):
        def decide(self, req, ctx):
            d = super().decide(req, ctx)
            d.request_id = "someone-else"
            return d

    rt = Mixed()
    _, gate = make_mock(runtime=rt)
    out = gate.call("read_file", J({"path": "a"}))
    assert not out.executed and rt.execute_calls == 0


def test_unknown_decision_value_is_denied(make_mock):
    class Odd(MockRuntime):
        def decide(self, req, ctx):
            d = super().decide(req, ctx)
            d.decision = "granted-by-vibes"
            return d

    rt = Odd()
    _, gate = make_mock(runtime=rt)
    out = gate.call("read_file", J({"path": "a"}))
    assert not out.executed and rt.execute_calls == 0
