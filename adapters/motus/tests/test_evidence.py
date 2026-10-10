import copy
import json
import subprocess
import sys

import pytest

from interplane.core import digest
from interplane_adapter_motus.evidence import seal_record, bundle_digest
from interplane_adapter_motus.ids import TRACE_RE
from interplane_adapter_motus.verify import main, verify_bundle

J = json.dumps


@pytest.fixture
def bundle(make_local):
    authority, gate = make_local()
    gate.call("read_file", J({"path": "notes.txt"}), motus_ids={"task_id": "7", "run_id": "run-1"})
    rid = gate.call("write_note", J({"path": "n.txt", "text": "x"})).record["request_id"]
    gate.approve(rid, lambda p: authority.issue_continuation(p, approved=True))
    gate.approve(rid, lambda p: authority.issue_continuation(p, approved=True))
    gate.call("read_file", "{bad")
    return gate.evidence()


def test_ids_agree_across_boundaries(bundle):
    assert TRACE_RE.fullmatch(bundle["trace_id"])
    recs = bundle["records"]
    assert {r["trace_id"] for r in recs} == {bundle["trace_id"]}
    assert recs[0]["motus"] == {"task_id": "7", "run_id": "run-1", "tool_call_id": None}
    assert "unavailable" in bundle["header"]["motus"]
    msg_ids = [r["message_id"] for r in recs if r["kind"] == "call" and r["message_id"]]
    assert len(msg_ids) == len(set(msg_ids))
    assert recs[0]["message_id"].startswith(f"motus-{bundle['trace_id']}-")
    assert [r["seq"] for r in recs] == list(range(1, len(recs) + 1))


def test_unavailable_evidence_is_labelled(bundle):
    bad = bundle["records"][-1]
    assert bad["capability_request_digest"] is None and "unavailable" in bad


def test_honest_bundle_verifies(bundle):
    assert verify_bundle(bundle) == []


def mutate(bundle, fn):
    b = copy.deepcopy(bundle)
    fn(b)
    return verify_bundle(b)


@pytest.mark.parametrize(
    "name, fn",
    [
        ("decision flipped", lambda b: b["records"][1].__setitem__("decision", "authorized")),
        ("result digest changed", lambda b: b["records"][0].__setitem__("result_digest", "sha256:" + "0" * 64)),
        ("record dropped", lambda b: b["records"].pop(0)),
        ("records swapped", lambda b: b["records"].reverse()),
        ("trace id changed", lambda b: b.__setitem__("trace_id", "0" * 32)),
        ("header changed", lambda b: b["header"].__setitem__("runtime_id", "other")),
        ("bundle digest changed", lambda b: b.__setitem__("bundle_digest", "sha256:" + "1" * 64)),
        ("record digest changed", lambda b: b["records"][0].__setitem__("record_digest", "sha256:" + "2" * 64)),
        ("record trace changed", lambda b: b["records"][0].__setitem__("trace_id", "f" * 32)),
        ("retry claimed", lambda b: b["records"][0].__setitem__("retried", True)),
        ("schema changed", lambda b: b.__setitem__("schema", "x")),
    ],
)
def test_tampering_fails_verification(bundle, name, fn):
    assert mutate(bundle, fn), name


def reseal(b):
    """A forger who recomputes every digest still fails the cross-field rules."""
    b["records"] = [seal_record(r) for r in b["records"]]
    b["bundle_digest"] = bundle_digest(b["schema"], b["trace_id"], b["header"],
                                       [r["record_digest"] for r in b["records"]])


def test_resealed_forgery_claiming_execution_without_authorization_fails(bundle):
    b = copy.deepcopy(bundle)
    held = next(r for r in b["records"] if r["outcome"] == "approval_pending")
    held["executed"] = True
    reseal(b)
    assert any("without an authorized decision" in p for p in verify_bundle(b))


def test_resealed_forgery_with_a_second_execution_fails(bundle):
    b = copy.deepcopy(bundle)
    cont = next(r for r in b["records"] if r["kind"] == "continuation" and r["executed"])
    twin = dict(cont, seq=len(b["records"]) + 1)
    b["records"].append(twin)
    reseal(b)
    assert any("second time" in p for p in verify_bundle(b))


def test_resealed_continuation_of_a_never_held_request_fails(bundle):
    b = copy.deepcopy(bundle)
    first = b["records"][0]
    forged = dict(first, kind="continuation", seq=len(b["records"]) + 1, request_id="mc-999999")
    b["records"].append(forged)
    reseal(b)
    assert any("never held" in p for p in verify_bundle(b))


def test_cli_verifies_and_detects_tampering(bundle, tmp_path):
    good = tmp_path / "good.json"
    good.write_text(J(bundle))
    bad_b = copy.deepcopy(bundle)
    bad_b["records"][0]["decision"] = "denied"
    bad = tmp_path / "bad.json"
    bad.write_text(J(bad_b))
    run = lambda p: subprocess.run([sys.executable, "-m", "interplane_adapter_motus.verify", str(p)],  # noqa: E731
                                   capture_output=True, text=True)
    ok, fail = run(good), run(bad)
    assert ok.returncode == 0 and ok.stdout.startswith("OK")
    assert fail.returncode == 1 and "FAIL" in fail.stdout
    assert run(tmp_path / "missing.json").returncode == 1
    assert main([]) == 2


def test_cli_rejects_non_finite_numbers(tmp_path):
    p = tmp_path / "nan.json"
    p.write_text('{"x": NaN}')
    assert main([str(p)]) == 1


def test_digest_is_canonical_sha256(bundle):
    rec = bundle["records"][0]
    body = {k: v for k, v in rec.items() if k != "record_digest"}
    assert rec["record_digest"] == digest(body) and rec["record_digest"].startswith("sha256:")
