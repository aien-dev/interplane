import copy
import json
import re
from pathlib import Path

import pytest

from interplane.core import (
    CapabilityDescriptor,
    CapabilityRequest,
    Catalog,
    Decision,
    Envelope,
    ErrorCode,
    Event,
    Exposure,
    InputRecord,
    Lifecycle,
    LifecycleError,
    Limits,
    Party,
    ProbeReport,
    ProtocolError,
    ProtocolVersion,
    RequestLedger,
    Selection,
    State,
    ToolRef,
    ToolRequest,
    ToolResult,
    digest,
    jcs,
    validate_envelope,
)
from conftest import CONF, load

SAMPLES = {
    Party: {"kind": "model", "id": "m", "trust": None, "x_party": 1},
    ToolRequest: {
        "kind": "tool_request",
        "request_id": "r",
        "tool": {"namespace": "a", "name": "b"},
        "arguments": {"k": [1, 2]},
        "provenance": {"dialect": "openai", "parser_version": "1.0.0"},
        "x_new": {"deep": True},
    },
    CapabilityRequest: {
        "kind": "capability_request",
        "request_id": "r",
        "runtime": "mock",
        "capability": "read_file",
        "arguments": {},
        "tool": {"name": "read_file"},
        "mapping": {"table_version": "1", "rule_id": "x"},
        "x_new": 2,
    },
    Decision: {
        "kind": "decision",
        "request_id": "r",
        "decision": "denied",
        "authority": {"runtime": "mock", "policy_engine": "p"},
        "reason": "no",
        "x_new": 3,
    },
    ToolResult: {
        "kind": "result",
        "request_id": "r",
        "status": "ok",
        "data": {"a": 1},
        "error": None,
        "x_new": 4,
    },
    Event: {"kind": "event", "event": "final", "seq": 0, "x_new": 5},
    Catalog: {
        "kind": "catalog",
        "runtime": "mock",
        "catalog_version": "1",
        "capabilities": [{"name": "n", "description": "d", "parameters": {}, "x_cap": 1}],
        "x_new": 6,
    },
    CapabilityDescriptor: {
        "name": "n",
        "description": "d",
        "parameters": {"type": "object"},
        "domains": ["fs"],
        "x_new": 7,
    },
    Selection: {
        "kind": "selection",
        "runtime": "mock",
        "catalog_digest": "sha256:" + "0" * 64,
        "selector": {"name": "domain_match", "version": "1"},
        "requested_domains": [],
        "selected": [],
        "excluded": [],
        "x_new": 8,
    },
    ProbeReport: {
        "kind": "probe_report",
        "probe_version": "0.1.0",
        "endpoint": "http://x",
        "model": "m",
        "started_at": "2026-01-01T00:00:00Z",
        "probes": [],
        "profiles": [],
        "x_new": 9,
    },
    ToolRef: {"namespace": None, "name": "read_file"},
}


@pytest.mark.parametrize("cls", list(SAMPLES))
def test_round_trip_and_unknown_fields(cls):
    sample = SAMPLES[cls]
    obj = cls.from_dict(copy.deepcopy(sample))
    assert obj.to_dict() == sample
    assert cls.from_dict(obj.to_dict()) == obj
    assert all(k in obj.extra for k in sample if k.startswith("x_"))


@pytest.mark.parametrize("cls", [c for c in SAMPLES if c is not ToolRef])
def test_malformed_input(cls):
    with pytest.raises(ProtocolError):
        cls.from_dict("not an object")
    with pytest.raises(ProtocolError):
        cls.from_dict({})


def test_wrong_kind_and_types():
    bad = dict(SAMPLES[Decision], kind="result")
    with pytest.raises(ProtocolError):
        Decision.from_dict(bad)
    with pytest.raises(ProtocolError):
        ToolRequest.from_dict(dict(SAMPLES[ToolRequest], arguments=[1]))
    with pytest.raises(ProtocolError) as e:
        ToolRequest.from_dict(dict(SAMPLES[ToolRequest], tool={"name": "bad name"}))
    assert e.value.code == ErrorCode.MALFORMED_TOOL_CALL
    with pytest.raises(ProtocolError):
        Event.from_dict(dict(SAMPLES[Event], seq=True))


def test_envelope_validation(envelope):
    assert validate_envelope(envelope()) is None
    assert validate_envelope("x") == ErrorCode.MALFORMED_ENVELOPE
    for key in ("message_id", "trace_id", "timestamp", "source", "destination", "payload"):
        env = envelope()
        del env[key]
        assert validate_envelope(env) == ErrorCode.MALFORMED_ENVELOPE, key
    assert validate_envelope(envelope(message_id="bad id!")) == ErrorCode.MALFORMED_ENVELOPE
    assert validate_envelope(envelope(message_id="x" * 129)) == ErrorCode.MALFORMED_ENVELOPE
    assert validate_envelope(envelope(message_id="ok\n")) == ErrorCode.MALFORMED_ENVELOPE
    assert validate_envelope(envelope(timestamp="yesterday")) == ErrorCode.MALFORMED_ENVELOPE
    assert (
        validate_envelope(envelope(source={"kind": "alien", "id": "x"}))
        == ErrorCode.MALFORMED_ENVELOPE
    )
    env = envelope()
    env["payload"]["kind"] = "bogus"
    assert validate_envelope(env) == ErrorCode.MALFORMED_ENVELOPE
    env = envelope()
    env["payload"]["arguments"] = [1]
    assert validate_envelope(env) == ErrorCode.MALFORMED_TOOL_CALL
    assert validate_envelope(envelope(digest="sha256:zz")) == ErrorCode.MALFORMED_ENVELOPE


def test_decision_enum_is_not_a_structural_error(envelope):
    env = envelope()
    env["payload"] = {"kind": "decision", "request_id": "r", "decision": "super_authorized"}
    assert validate_envelope(env) is None  # receivers treat unknown values as denied, not malformed
    assert Decision.from_dict(SAMPLES[Decision] | {"decision": "weird"}).is_authorized is False


def test_versions(envelope):
    assert ProtocolVersion.parse("0.7").check() is None
    assert ProtocolVersion.parse("1.0").check() == ErrorCode.UNSUPPORTED_VERSION
    assert validate_envelope(envelope(interplane_version="1.0")) == ErrorCode.UNSUPPORTED_VERSION
    assert validate_envelope(envelope(interplane_version="abc")) == ErrorCode.MALFORMED_ENVELOPE
    assert validate_envelope(envelope(interplane_version=1.0)) == ErrorCode.MALFORMED_ENVELOPE
    # a MAJOR mismatch wins over other defects: the rest is not read
    env = envelope(interplane_version="3.1")
    del env["payload"]
    assert validate_envelope(env) == ErrorCode.UNSUPPORTED_VERSION


def test_envelope_object_preserves_everything(envelope):
    env = envelope(x_top={"a": 1}, extensions={"k": "v"}, signature=None)
    obj = Envelope.from_dict(env)
    assert obj.to_dict() == env
    assert obj.payload_object().request_id == "r-1"


def test_error_codes_match_schema():
    schema = load(CONF.parents[1] / "spec" / "schemas" / "common.schema.json")
    assert ErrorCode.ALL == set(schema["$defs"]["ErrorCode"]["enum"])


# ---- JCS ----------------------------------------------------------------------------------


@pytest.mark.parametrize(
    "value,text",
    [
        (1.0, "1"),
        (-0.0, "0"),
        (1e21, "1e+21"),
        (1e20, "100000000000000000000"),
        (0.5, "0.5"),
        (1e-7, "1e-7"),
        (0.000001, "0.000001"),
        (123456789.125, "123456789.125"),
        (2.50, "2.5"),
        (1.5e300, "1.5e+300"),
        (5e-324, "5e-324"),
        (True, "true"),
        (None, "null"),
        (12, "12"),
        ("é\n\x01\x7f", '"é\\n\\u0001\x7f"'),
        ({"b": 1, "a": [1, {"z": 0, "y": 0}]}, '{"a":[1,{"y":0,"z":0}],"b":1}'),
    ],
)
def test_jcs_vectors(value, text):
    assert jcs(value) == text


def test_jcs_rejects_non_finite():
    for bad in (float("nan"), float("inf"), float("-inf")):
        with pytest.raises(ValueError):
            jcs(bad)
    with pytest.raises(TypeError):
        jcs({1: 2})


def test_jcs_utf16_key_order():
    assert jcs({"～": 1, "\U0001f600": 2}) == '{"\U0001f600":2,"～":1}'


def test_digest_format():
    assert re.fullmatch(r"sha256:[0-9a-f]{64}", digest({"a": 1}))
    assert digest({"a": 1, "b": 2}) == digest({"b": 2, "a": 1})


def test_jcs_fixture():
    path = CONF / "digest" / "jcs-01.json"
    if not path.exists():
        pytest.skip("fixture absent")
    fx = json.loads(path.read_text(encoding="utf-8"))
    assert jcs(fx["value"]) == fx["expected_jcs"]
    assert digest(fx["value"]) == fx["expected_sha256"]


def test_digest_fixtures_all_pass():
    from interplane.conformance import check_digest_fixtures

    results = check_digest_fixtures(str(CONF))
    assert len(results) >= 2
    for name, err in results:
        assert err is None, name


def test_input_record_examples_roundtrip_and_null_parent_is_explicit():
    files = sorted((CONF / "input").glob("*.json"))
    assert len(files) == 9
    for f in files:
        raw = json.loads(f.read_text(encoding="utf-8"))
        assert InputRecord.from_dict(raw).to_dict() == raw
    rec = InputRecord(
        input_id="i",
        content_kind="memory",
        trust="made_up",
        source=Party("runtime", "r"),
        origin="m",
        content_digest="sha256:00",
        trace_id="t",
    )
    out = rec.to_dict()
    assert out["parent_id"] is None and out["derived_from"] == [] and out["trust"] == "made_up"
    with pytest.raises(ProtocolError):
        InputRecord.from_dict({k: v for k, v in out.items() if k != "origin"})


def test_exposure_roundtrip_and_requires_both_fields():
    raw = {"inputs": ["a"], "floor": "unknown", "x": 1}
    assert Exposure.from_dict(raw).to_dict() == raw
    with pytest.raises(ProtocolError):
        Exposure.from_dict({"inputs": ["a"]})


# ---- limits and ledger ----------------------------------------------------------------------


def test_limits_defaults_and_override():
    lim = Limits()
    assert (lim.max_argument_bytes, lim.max_requests_per_turn) == (65536, 32)
    assert Limits.from_dict({"max_argument_bytes": 10, "junk": 1}).max_argument_bytes == 10
    assert Limits(max_argument_bytes=10).arguments_oversized({"a": "x" * 20})
    assert not Limits().arguments_oversized({"a": "x"})


def test_ledger_replay_and_duplicates():
    led = RequestLedger()
    assert led.admit("t", "m1", "r1") is None
    assert led.admit("t", "m1", "r1") == ErrorCode.REPLAYED_MESSAGE
    assert led.admit("t", "m2", "r1") == ErrorCode.DUPLICATE_REQUEST_ID
    assert led.admit("other", "m1", "r1") is None  # ids are scoped per trace


# ---- lifecycle ------------------------------------------------------------------------------


def dec(value, rid="r", approval=None):
    return Decision(
        request_id=rid,
        decision=value,
        authority={"runtime": "x", "policy_engine": "y"},
        approval=approval,
    )


def mapped():
    lc = Lifecycle("r")
    lc.map()
    return lc


@pytest.mark.parametrize(
    "value,state",
    [
        ("authorized", State.AUTHORIZED),
        ("denied", State.DENIED),
        ("requires_approval", State.REQUIRES_APPROVAL),
        ("not_found", State.REJECTED),
        ("invalid", State.REJECTED),
        ("surprise", State.DENIED),
        ("AUTHORIZED", State.DENIED),
    ],
)
def test_decision_transitions(value, state):
    lc = mapped()
    assert lc.apply_decision(dec(value)) is state


def test_happy_path_and_terminals():
    lc = mapped()
    lc.apply_decision(dec("authorized"))
    assert lc.start_execution() is State.EXECUTING
    assert lc.finish("ok") is State.SUCCEEDED
    for call in (
        lc.map,
        lc.reject,
        lc.start_execution,
        lambda: lc.finish("ok"),
        lambda: lc.apply_decision(dec("authorized")),
    ):
        with pytest.raises(LifecycleError):
            call()
    for status, st in (("error", State.FAILED), ("timed_out", State.TIMED_OUT)):
        lc = mapped()
        lc.apply_decision(dec("authorized"))
        lc.start_execution()
        assert lc.finish(status) is st


def test_denied_never_becomes_authorized():
    lc = mapped()
    lc.apply_decision(dec("denied"))
    with pytest.raises(LifecycleError):
        lc.apply_decision(dec("authorized"))
    assert lc.state is State.DENIED
    with pytest.raises(LifecycleError):
        lc.start_execution()


def test_requires_approval_needs_new_decision_citing_id():
    lc = mapped()
    lc.apply_decision(dec("requires_approval", approval={"approval_id": "A1"}))
    for bad in (
        dec("authorized"),
        dec("authorized", approval={"approval_id": "forged"}),
        dec("authorized", approval={"approval_id": ""}),
    ):
        with pytest.raises(LifecycleError):
            lc.apply_decision(bad)
        assert lc.state is State.REQUIRES_APPROVAL
    lc.apply_decision(dec("authorized", approval={"approval_id": "A1"}))
    assert lc.state is State.AUTHORIZED
    lc2 = mapped()
    lc2.apply_decision(dec("requires_approval", approval={"approval_id": "A1"}))
    lc2.apply_decision(dec("denied", approval={"approval_id": "A1"}))
    assert lc2.state is State.DENIED


def test_no_public_path_to_authorized_without_runtime_decision():
    # PROPOSED / MAPPED cannot execute and cannot be pushed to AUTHORIZED by any other method.
    public = [n for n in dir(Lifecycle) if not n.startswith("_")]
    assert sorted(public) == [
        "apply_decision",
        "decision",
        "finish",
        "map",
        "reject",
        "request_id",
        "start_execution",
        "state",
    ]
    for prep in (lambda: Lifecycle("r"), mapped):
        lc = prep()
        with pytest.raises(LifecycleError):
            lc.start_execution()
        with pytest.raises(LifecycleError):
            lc.finish("ok")
        with pytest.raises(AttributeError):
            lc.state = State.AUTHORIZED
        for junk in (
            None,
            {"decision": "authorized"},
            "authorized",
            dec("authorized", rid="other"),
        ):
            with pytest.raises(LifecycleError):
                lc.apply_decision(junk)
        assert lc.state in (State.PROPOSED, State.MAPPED)
    lc = Lifecycle("r")
    with pytest.raises(LifecycleError):
        lc.apply_decision(dec("authorized"))  # must be MAPPED first


def test_interplane_code_never_constructs_authorized_decisions():
    src = Path(__file__).resolve().parents[1] / "interplane"
    offenders = []
    for path in src.rglob("*.py"):
        if path.name == "conformance_negctl.py":
            continue  # test-only negative controls V2/V4 forge authorizations on purpose; never imported by the library
        text = path.read_text(encoding="utf-8")
        if path.name == "crossveil.py":
            text = text.split("class MockRuntime", 1)[0] + text.split("def default_pipeline", 1)[-1]
        for m in re.finditer(r"Decision\([^)]*\"authorized\"", text):
            offenders.append(path.name)
        if path.name != "core.py" and re.search(r"decision\s*=\s*\"authorized\"", text):
            offenders.append(path.name)
    assert offenders == []
