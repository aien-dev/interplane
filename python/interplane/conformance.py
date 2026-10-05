"""Conformance runner: ``interplane-conformance <fixtures_dir> --out <verdicts.json>``."""

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

from .core import (
    Decision,
    Exposure,
    InputRecord,
    Lifecycle,
    LifecycleError,
    Limits,
    ProtocolError,
    ToolResult,
    digest,
    jcs,
)
from .crossaxis import expand, select
from .crossveil import canonical_result_payload, default_pipeline


def _dig(obj: Any, path: str) -> Any:
    for part in path.split("."):
        if isinstance(obj, list):
            obj = obj[int(part)]
        elif isinstance(obj, dict) and part in obj:
            obj = obj[part]
        else:
            raise KeyError(path)
    return obj


def run_case(case: dict) -> dict:
    """Run one fixture on a fresh pipeline and mock runtime; returns the verdict."""
    pipe = default_pipeline(
        Limits.from_dict(case.get("limits")), case.get("mapping_table", "mock-table")
    )
    pipe.runtime.provenance_overrides = case.get("mock_provenance", {})
    name, trace_id = case["case"], case["trace_id"]
    observed: list = []
    results: list = []
    turns: list = []
    selections: list = []
    sel_spec = case.get("selection")
    catalog = pipe.runtime.catalog()
    sel = None
    if sel_spec is not None:
        sel, _ = select(
            catalog,
            sel_spec.get("requested_domains", []),
            sel_spec.get("max_capabilities"),
            sel_spec.get("always_include"),
        )
    for step in case["steps"]:
        if "expand" in step:
            ex = step["expand"]
            sel = expand(
                sel,
                catalog,
                ex["evidence"],
                ex.get("max_expansions"),
                ex.get("max_added_per_expansion"),
            )
            selections.append(sel.to_dict())
            continue
        if "input" in step and "dialect" not in step:
            # host-only input registration (0.3 cut P3): never reachable from model input
            try:
                pipe.register_input(step["input"])
            except (ValueError, ProtocolError):
                pass
            continue
        if "envelope" in step:
            result, record = pipe.admit_envelope(step["envelope"], turn=step.get("turn"))
            results.append(result)
            observed.append(record.to_dict())
            continue
        out = pipe.run_turn(
            step["dialect"],
            step.get("model", ""),
            step["input"],
            trace_id,
            step["turn"],
            message_id_prefix=f"m-{name}-{step['turn']}",
        )
        results.extend(out.results)
        observed.extend(r.to_dict() for r in out.observed)
        turns.append(out.to_dict())
    # Reported only for cases that assert them, so older verdicts keep their shape.
    inputs = pipe.inputs(trace_id) if "inputs" in case["expected"] else []
    exposure = pipe.runtime.seen_exposure if "exposure" in case["expected"] else []
    runtime = {
        "decide_calls": pipe.runtime.decide_calls,
        "execute_calls": pipe.runtime.execute_calls,
    }

    expected = case["expected"]
    problems: list = []
    exp_obs = expected.get("observed", [])
    if len(exp_obs) != len(observed):
        problems.append(f"observed count {len(observed)} != {len(exp_obs)}")
    for i, (got, want) in enumerate(zip(observed, exp_obs)):
        for key, value in want.items():
            if key == "result_digest" and value is None:
                continue
            if got.get(key) != value:
                problems.append(f"observed[{i}].{key}: {got.get(key)!r} != {value!r}")
    if "turns" in expected and turns != expected["turns"]:
        problems.append(f"turns {turns!r} != {expected['turns']!r}")
    if "selections" in expected and selections != expected["selections"]:
        problems.append("selections differ from expected")
    if "runtime" in expected and runtime != expected["runtime"]:
        problems.append(f"runtime {runtime!r} != {expected['runtime']!r}")
    if "inputs" in expected:
        want = expected["inputs"]
        ok = len(want) == len(inputs) and all(
            all(g.get(k) == v for k, v in w.items()) for w, g in zip(want, inputs)
        )
        if not ok:
            problems.append(f"inputs {inputs!r} != {want!r}")
    if "exposure" in expected and exposure != expected["exposure"]:
        problems.append(f"exposure {exposure!r} != {expected['exposure']!r}")
    for check in expected.get("result_checks", []):
        match = next((r for r in results if r.request_id == check["request_id"]), None)
        try:
            payload = match.to_dict()
            if check.get("round_trip") is True:
                # as a receiver sees it: JCS bytes parsed back into the SDK's own result type
                payload = ToolResult.from_dict(json.loads(jcs(payload))).to_dict()
            actual = _dig(payload, check["path"])
        except (AttributeError, KeyError, IndexError, ValueError):
            problems.append(f"result_check {check['request_id']} {check['path']}: missing")
            continue
        if actual != check["equals"]:
            problems.append(f"result_check {check['request_id']} {check['path']}: {actual!r}")
    return {
        "pass": not problems,
        "observed": observed,
        "turns": turns,
        "runtime": runtime,
        "selections": selections,
        "inputs": inputs,
        "exposure": exposure,
        "problems": problems,
        "results": [canonical_result_payload(r) for r in results],
    }


def run_lifecycle_case(case: dict) -> dict:
    """Drive the Crossveil lifecycle directly (no pipeline). For each step, the lifecycle of
    ``request_id`` (created and mapped on first sight) receives ``decision``; the step records the
    state after the call and whether the call was refused."""
    lifecycles: dict = {}
    steps: list = []
    for step in case["steps"]:
        rid = step["request_id"]
        life = lifecycles.get(rid)
        if life is None:
            life = lifecycles[rid] = Lifecycle(rid)
            life.map()
        try:
            life.apply_decision(Decision.from_dict(step["decision"]))
            refused = False
        except (LifecycleError, ProtocolError):
            refused = True
        steps.append({"request_id": rid, "state": life.state.value, "refused": refused})
    problems = [] if steps == case["expected"] else [f"steps {steps!r} != {case['expected']!r}"]
    return {"pass": not problems, "steps": steps, "problems": problems}


def typed_roundtrip(type_name: str, value: Any) -> Any:
    """Parse ``value`` as the named SDK type and serialize it back."""
    types = {"InputRecord": InputRecord, "Exposure": Exposure}
    if type_name not in types:
        raise ValueError(f"unknown fixture type {type_name}")
    return types[type_name].from_dict(value).to_dict()


def check_digest_fixtures(fixtures_dir: str) -> list:
    """Check every ``digest/*.json``. A fixture with a ``type`` is parsed as that SDK type and
    re-serialized before the JCS and digest are compared. Returns ``[(name, error or None)]``."""
    out = []
    for path in sorted(Path(fixtures_dir, "digest").glob("*.json")):
        fx = json.loads(path.read_text(encoding="utf-8"))
        err = None
        try:
            value = fx["value"]
            if "type" in fx:
                value = typed_roundtrip(fx["type"], value)
            if jcs(value) != fx["expected_jcs"]:
                err = f"jcs mismatch: got {jcs(value)}"
            elif digest(value) != fx["expected_sha256"]:
                err = "sha256 mismatch"
        except (ValueError, ProtocolError) as e:
            err = str(e)
        out.append((f"digest/{path.stem}", err))
    return out


def main(argv: Optional[list] = None) -> int:
    ap = argparse.ArgumentParser(prog="interplane-conformance")
    ap.add_argument("fixtures_dir")
    ap.add_argument("--out", required=True)
    ap.add_argument("--dump", help="directory for per-case canonical result payloads")
    args = ap.parse_args(argv)
    verdicts: dict = {}
    for path in sorted(Path(args.fixtures_dir).glob("*.json")):
        case = json.loads(path.read_text(encoding="utf-8"))
        verdict = run_case(case)
        verdicts[case["case"]] = verdict
        if args.dump:
            d = Path(args.dump)
            d.mkdir(parents=True, exist_ok=True)
            (d / f"{case['case']}.results.json").write_text(
                jcs(verdict["results"]) + "\n", encoding="utf-8"
            )
    lifecycle: dict = {}
    for path in sorted(Path(args.fixtures_dir, "lifecycle").glob("*.json")):
        case = json.loads(path.read_text(encoding="utf-8"))
        lifecycle[case["case"]] = run_lifecycle_case(case)
    failed = 0
    for name, v in sorted({**verdicts, **lifecycle}.items()):
        print(f"{'PASS' if v['pass'] else 'FAIL'}  {name}")
        for p in v["problems"]:
            print(f"      {p}")
        failed += not v["pass"]
    for name, err in check_digest_fixtures(args.fixtures_dir):
        print(f"{'PASS' if err is None else 'FAIL'}  {name}")
        if err:
            print(f"      {err}")
        failed += err is not None
    total = len(verdicts) + len(lifecycle)
    print(f"{total - failed}/{total} passed")
    fields = ("pass", "observed", "turns", "runtime", "selections", "inputs", "exposure")
    out = {k: {f: v[f] for f in fields if f in v} for k, v in verdicts.items()}
    for v in out.values():
        for key in ("selections", "inputs", "exposure"):
            if not v.get(key):
                v.pop(key, None)
    out.update({k: {"pass": v["pass"], "steps": v["steps"]} for k, v in lifecycle.items()})
    Path(args.out).write_text(jcs(out) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
