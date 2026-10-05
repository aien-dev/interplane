import pytest

jsonschema = pytest.importorskip("jsonschema")

from interplane.crossveil import default_pipeline  # noqa: E402
from interplane.crossaxis import select  # noqa: E402
from interplane.validate import errors, validate  # noqa: E402
from interplane import lenshift  # noqa: E402
from conftest import CONF, DIALECTS, load  # noqa: E402
import json  # noqa: E402


def test_validator_has_teeth():
    assert errors(
        "decision",
        {
            "kind": "decision",
            "request_id": "r",
            "decision": "yes",
            "authority": {"runtime": "a", "policy_engine": "b"},
        },
    )
    assert errors("result", {"kind": "result", "request_id": "r", "status": "denied", "data": None})


def run_cases():
    for path in sorted(CONF.glob("*.json")):
        case = load(path)
        pipe = default_pipeline()
        results = []
        for step in case["steps"]:
            if "expand" in step:
                continue
            if "input" in step and "dialect" not in step:
                pipe.register_input(step["input"])
            elif "envelope" in step:
                results.append(pipe.admit_envelope(step["envelope"])[0])
            else:
                results.extend(
                    pipe.run_turn(
                        step["dialect"],
                        step.get("model", ""),
                        step["input"],
                        case["trace_id"],
                        step["turn"],
                    ).results
                )
        yield case["case"], results


def test_every_conformance_result_and_decision_validates():
    seen = 0
    for name, results in run_cases():
        for res in results:
            d = res.to_dict()
            if d["request_id"] is None:
                continue  # null id before an id existed (case 13): not schema-valid by design
            validate("result", d)
            seen += 1
            if res.decision is not None:
                validate("decision", res.decision.to_dict())
    assert seen >= 15


def test_decisions_capability_requests_catalog_selection_validate():
    p = default_pipeline()
    rt = p.runtime
    out = p.run_turn(
        "openai",
        "m",
        {
            "tool_calls": [
                {"id": "c", "function": {"name": "filesystem.read", "arguments": '{"path":"p"}'}}
            ]
        },
        "t",
        0,
    )
    intent = (
        lenshift.get("openai")
        .parse(
            {
                "tool_calls": [
                    {
                        "id": "c",
                        "function": {"name": "filesystem.read", "arguments": '{"path":"p"}'},
                    }
                ]
            },
            "m",
            "t",
            0,
        )
        .intents[0]
    )
    validate("intent", intent.to_dict())
    cap = p.mapping_table.map(intent)
    validate("capability", cap.to_dict())
    for name, args in (
        ("read_file", {"path": "p"}),
        ("write_file", {"path": "p", "content": "c"}),
        ("delete_file", {"path": "p"}),
        ("nope", {}),
    ):
        cap.capability, cap.arguments = name, args
        validate("decision", rt.decide(cap, {}).to_dict())
    validate("result", out.results[0].to_dict())
    cat = rt.catalog()
    validate("catalog", cat.to_dict())
    validate("selection", select(cat, ["filesystem"], 3, ["fail_tool"])[0].to_dict())


def test_expansion_receipts_and_measure_validate():
    from interplane.crossaxis import EstimatedTokens, expand, openai_tools_renderer

    cat = default_pipeline().runtime.catalog()
    sel, caps = select(cat, ["email"], 3, ["fail_tool"], renderer=openai_tools_renderer)
    validate("selection", sel.to_dict())
    from interplane.crossaxis import measure

    msgs = [{"role": "user", "content": "hi"}]
    sel.measure = measure(cat.capabilities, caps, openai_tools_renderer, EstimatedTokens(), msgs)
    validate("selection", sel.to_dict())
    out = expand(sel, cat, {"kind": "requested_excluded", "name": "read_file"})
    out = expand(out, cat, {"kind": "discovery_hit", "names": ["web_fetch", "nope"]})
    validate("selection", out.to_dict())
    for path in sorted(CONF.glob("2[45]-*.json")):
        for s in load(path)["expected"]["selections"]:
            validate("selection", s)
    bad = out.to_dict()
    bad["expansions"][0]["refused"] = [{"name": "x"}]
    assert errors("selection", bad)
    bad = sel.to_dict()
    bad["measure"]["tokens"]["unit"] = "tokens"
    assert errors("selection", bad)


def test_envelopes_from_pipeline_validate():
    for path in sorted(CONF.glob("*.json")):
        for step in load(path)["steps"]:
            if "envelope" in step and step["envelope"]["interplane_version"] == "1.0":
                validate("envelope", step["envelope"])


def test_dialect_intents_validate():
    for path in DIALECTS.glob("*/*.json"):
        fx = load(path)
        turn = lenshift.get(fx["dialect"]).parse(
            fx["input"], fx["model"], fx["trace_id"], fx["turn"]
        )
        for intent in turn.intents:
            validate("intent", json.loads(json.dumps(intent.to_dict())))
