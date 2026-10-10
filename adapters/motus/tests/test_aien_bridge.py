"""AienBridgeAuthority: fail-closed behaviour with fake bridges, and the real AIEN path end to end.

The fake bridges are tiny scripts that speak the same line protocol. The end-to-end class runs the
real ``aien-authority-bridge`` binary (adapters/aien) and is skipped, with a reason, when it is not built.
"""

import json
import os
import stat
import sys
import textwrap
from pathlib import Path

import pytest

from interplane.core import CapabilityRequest

from interplane_adapter_motus.aien_bridge import ENV_BRIDGE, UNAVAILABLE, AienBridgeAuthority
from interplane_adapter_motus.gate import Gate
from interplane_adapter_motus.local_authority import build_catalog
from interplane_adapter_motus.mapping import mapping_for
from interplane_adapter_motus.verify import verify_bundle

REPO = Path(__file__).resolve().parents[3]
BUILT = REPO / "adapters" / "aien" / "target" / "debug" / "aien-authority-bridge"
TRACE = "0123456789abcdef0123456789abcdef"
CTX = {"trace_id": TRACE, "message_id": "m-1", "parent_id": None,
       "model": {"kind": "model", "id": "t"}, "exposure": None}


def real_binary():
    for cand in (os.environ.get(ENV_BRIDGE), str(BUILT)):
        if cand and os.access(cand, os.X_OK):
            return cand
    return None


def req(rid="r-1", cap="read_file", **args):
    return CapabilityRequest.from_dict({
        "kind": "capability_request", "request_id": rid, "runtime": "aien", "capability": cap,
        "arguments": args or {"path": "notes.txt"}, "tool": {"namespace": None, "name": cap},
        "mapping": {"table_version": "1", "rule_id": f"passthrough:{cap}", "passthrough": True},
    })


def fake_bridge(tmp_path, decide_body):
    """A bridge that answers catalog correctly and runs `decide_body` (python source using
    `msg`, `reply(obj)` and `sys`) for every other op."""
    cat = build_catalog().to_dict()
    cat["runtime"] = "aien"
    script = tmp_path / "fake-bridge"
    script.write_text(
        f"#!{sys.executable}\n"
        "import json, sys, time\n"
        "def reply(o):\n    sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n"
        f"CAT = {json.dumps(cat)!r}\n"
        "for line in sys.stdin:\n"
        "    msg = json.loads(line)\n"
        "    if msg['op'] == 'catalog':\n"
        "        reply({'ok': True, 'catalog': json.loads(CAT)}); continue\n"
        + textwrap.indent(textwrap.dedent(decide_body), "    ")
        + "\n"
    )
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    return str(script)


def authorized_reply():
    return textwrap.dedent("""
        if msg['op'] == 'decide':
            r = msg['request']
            reply({'ok': True, 'decision': {'kind': 'decision', 'request_id': r['request_id'],
                'decision': 'authorized', 'capability': r['capability'],
                'authority': {'runtime': 'aien', 'policy_engine': 'fake', 'decision_id': None},
                'reason': None, 'constraints': []}})
        """)


class TestFailClosed:
    def test_missing_binary_denies_and_never_executes(self, tmp_path):
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=str(tmp_path / "nope"))
        assert not auth.available
        d = auth.decide(req(), CTX)
        assert (d.decision, d.reason) == ("denied", UNAVAILABLE)
        assert auth.execute_calls == 0

    def test_no_configured_path_denies(self, tmp_path, monkeypatch):
        monkeypatch.delenv(ENV_BRIDGE, raising=False)
        auth = AienBridgeAuthority(str(tmp_path))
        assert auth.decide(req(), CTX).reason == UNAVAILABLE

    def test_gate_with_unavailable_bridge_executes_nothing(self, tmp_path):
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=str(tmp_path / "nope"))
        gate = Gate(auth, mapping_for(auth.catalog(), auth.runtime_id))
        gate.register_user_turn("x")
        out = gate.call("read_file", json.dumps({"path": "notes.txt"}))
        assert not out.executed and auth.execute_calls == 0
        assert out.outcome in ("rejected", "unavailable")

    def test_malformed_json_fails_closed(self, tmp_path):
        path = fake_bridge(tmp_path, "sys.stdout.write('this is not json\\n'); sys.stdout.flush()")
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        assert auth.available
        d = auth.decide(req(), CTX)
        assert (d.decision, d.reason) == ("denied", UNAVAILABLE)
        assert not auth.available
        assert auth.decide(req("r-2"), CTX).decision == "denied"
        assert auth.execute_calls == 0

    def test_decision_for_a_different_request_is_rejected(self, tmp_path):
        body = authorized_reply().replace("r['request_id'],\n", "'someone-else',\n", 1)
        path = fake_bridge(tmp_path, body)
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        d = auth.decide(req("r-1"), CTX)
        assert d.decision == "denied" and d.reason == UNAVAILABLE and d.request_id == "r-1"
        assert auth.fault == "decide: decision is for a different request"

    def test_timeout_fails_closed(self, tmp_path):
        path = fake_bridge(tmp_path, "time.sleep(30)")
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=0.5)
        d = auth.decide(req(), CTX)
        assert (d.decision, d.reason) == ("denied", UNAVAILABLE)
        assert auth.fault == "timeout"

    def test_exit_in_the_middle_of_decide_fails_closed(self, tmp_path):
        path = fake_bridge(tmp_path, "sys.exit(3)")
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        assert auth.decide(req(), CTX).reason == UNAVAILABLE

    def test_bridge_error_reply_is_never_a_decision(self, tmp_path):
        path = fake_bridge(tmp_path, "reply({'ok': False, 'error': 'nope'})")
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        assert auth.decide(req(), CTX).decision == "denied"

    def test_exit_during_execute_is_reported_uncertain_not_retried(self, tmp_path):
        path = fake_bridge(tmp_path, authorized_reply() + "\nif msg['op'] == 'execute':\n    sys.exit(1)\n")
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        r = req()
        d = auth.decide(r, CTX)
        assert d.decision == "authorized"
        res = auth.execute(r, d, CTX)
        assert res.status == "error" and res.error.message.startswith("uncertain effect")
        assert auth.execute_calls == 1
        assert auth.execute(r, d, CTX).error.code == "runtime_unavailable"

    def test_result_for_a_different_request_is_rejected(self, tmp_path):
        body = authorized_reply() + textwrap.dedent("""
            if msg['op'] == 'execute':
                reply({'ok': True, 'result': {'kind': 'result', 'request_id': 'other', 'status': 'ok', 'data': {}}})
            """)
        path = fake_bridge(tmp_path, body)
        auth = AienBridgeAuthority(str(tmp_path), bridge_path=path, timeout=5)
        r = req()
        res = auth.execute(r, auth.decide(r, CTX), CTX)
        assert res.status == "error" and res.request_id == "r-1"


@pytest.mark.skipif(real_binary() is None, reason="aien-authority-bridge not built: run `cargo build --bin aien-authority-bridge` in adapters/aien")
class TestRealAien:
    @pytest.fixture
    def rig(self, tmp_path):
        ws = tmp_path / "ws"
        ws.mkdir()
        (ws / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
        (tmp_path / "outside.txt").write_text("SECRET\n", encoding="utf-8")
        auth = AienBridgeAuthority(str(ws), bridge_path=real_binary(), timeout=20)
        assert auth.available, auth.fault
        gate = Gate(auth, auth.mapping_table())
        gate.register_user_turn("test request")
        yield ws, auth, gate
        auth.close()

    def test_authorized_read_executes(self, rig):
        ws, auth, gate = rig
        out = gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="c-1")
        assert (out.outcome, out.executed) == ("finished", True)
        assert "hello from the workspace" in out.text
        assert gate.evidence()["header"]["runtime_id"] == "aien"

    def test_path_escape_denied_and_nothing_read(self, rig):
        ws, auth, gate = rig
        out = gate.call("read_file", json.dumps({"path": "../outside.txt"}), call_id="c-2")
        assert (out.outcome, out.executed) == ("rejected", False)
        assert "SECRET" not in out.text
        assert out.record["authority_execute_calls"] == 0

    def test_malformed_arguments_rejected(self, rig):
        out = rig[2].call("read_file", "{not json", call_id="c-3")
        assert (out.outcome, out.executed) == ("rejected", False)

    def test_duplicate_request_id_rejected(self, rig):
        gate = rig[2]
        assert gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="c-4").executed
        dup = gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="c-4")
        assert (dup.executed, dup.record["error_code"]) == (False, "duplicate_request_id")

    def test_write_held_then_single_use_continuation(self, rig):
        ws, auth, gate = rig
        held = gate.call("write_file", json.dumps({"path": "copy.txt", "content": "x"}), call_id="c-5")
        assert held.outcome == "approval_pending" and not (ws / "copy.txt").exists()
        first = gate.approve("c-5", lambda p: auth.issue_continuation(p, approved=True))
        assert (first.outcome, first.executed) == ("finished", True)
        assert (ws / "copy.txt").read_text() == "x"
        second = gate.approve("c-5", lambda p: auth.issue_continuation(p, approved=True))
        assert not second.executed

    def test_evidence_verifies_and_tamper_fails(self, rig):
        gate = rig[2]
        gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="c-6")
        gate.call("read_file", json.dumps({"path": "../outside.txt"}), call_id="c-7")
        bundle = json.loads(json.dumps(gate.evidence()))
        assert verify_bundle(bundle) == []
        bundle["records"][1]["decision"] = "authorized"
        assert verify_bundle(bundle) != []

    def test_killed_bridge_fails_closed_mid_run(self, rig):
        ws, auth, gate = rig
        auth._proc.kill()
        out = gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="c-8")
        assert not out.executed
