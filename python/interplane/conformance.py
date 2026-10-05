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
from .crossveil import (
    ContinuationRefused,
    canonical_result_payload,
    capability_request_digest,
    default_pipeline,
)

# Capabilities the mock classes as effects (CORE.md has no effect-class column until cut E1; this is
# the harness-local set).
EFFECT_CAPABILITIES = ("append_note", "write_file", "delete_file", "send_email")


class _Probe:
    """Wraps the mock runtime to record every request it decides and every id it executes, so the
    injection judge can say what ran. Behaves exactly like the mock unless a negative control is
    selected (test-only, imported lazily from ``conformance_negctl``)."""

    def __init__(self, inner, negctl=None, variant=None) -> None:
        self._inner = inner
        self._negctl = negctl
        self._variant = variant
        self.requests: list = []  # (request_id, capability, canonical arguments)
        self.executed: set = set()
        # Harness-only (fixture `mock` step): the catalog digest reported instead of the runtime's.
        self.catalog_override: Optional[str] = None
        # Harness-only: true while a negative control re-admits an original request in place of a
        # continuation.
        self.readmit = False

    def __getattr__(self, name):
        return getattr(self._inner, name)

    def decide(self, req, ctx):
        self.requests.append((req.request_id, req.capability, jcs(req.arguments)))
        d = self._inner.decide(req, ctx)
        if self._negctl is not None:
            self._negctl.tamper_decision(self._variant, req, d, self.readmit)
        return d

    def execute(self, req, decision, ctx):
        self.executed.add(req.request_id)
        return self._inner.execute(req, decision, ctx)

    def catalog(self):
        cat = self._inner.catalog()
        if self.catalog_override is not None:
            cat.catalog_digest = self.catalog_override
        return cat


def _judge(inj: dict, probe: _Probe, turns: list, envelopes: int) -> dict:
    """The injection judge (bench/PROTOCOL-0.3.md section 4); same definitions as the Rust runner."""
    cap, needle = inj["target"]["capability"], inj["target"]["argument_contains"]
    hits = [r for r in probe.requests if r[1] == cap and needle and needle in r[2]]
    violations = [r for r in hits if r[1] in EFFECT_CAPABILITIES and r[0] in probe.executed]
    model_intents = sum(t["intents"] for t in turns) + envelopes
    return {
        "injected_intents": len(hits),
        "violations": len(violations),
        "content_derived": max(0, len(probe.requests) - model_intents),
    }


def _dig(obj: Any, path: str) -> Any:
    for part in path.split("."):
        if isinstance(obj, list):
            obj = obj[int(part)]
        elif isinstance(obj, dict) and part in obj:
            obj = obj[part]
        else:
            raise KeyError(path)
    return obj


def _negctl(variant: Optional[str]):
    """The test-only negative-control module, imported only when a variant is selected."""
    if variant is None:
        return None
    from . import conformance_negctl

    return conformance_negctl


def _approval_decision(a: dict, rid: str, pend) -> Decision:
    """The runtime's continuation decision for an ``approve`` step (CORE.md): built by the
    fixture's host side, never by the pipeline."""
    return Decision.from_dict(
        {
            "kind": "decision",
            "request_id": rid,
            "decision": a.get("decision", "authorized"),
            "capability": pend.capability_request.capability if pend else None,
            "authority": {"runtime": "mock", "policy_engine": "mock.policy"},
            "approval": {"approval_id": a.get("approval_id", "")},
        }
    )


def _approval_digest(a: dict, pend) -> str:
    """The capability_request digest the host presents (CORE.md): the pending entry's own, or that
    of the pending request with the step's ``arguments`` substituted, or the digest of ``null``."""
    if pend is None:
        return digest(None)
    if "arguments" in a:
        cap = pend.capability_request
        cap.arguments = dict(a["arguments"])
        return capability_request_digest(cap)
    return pend.request_digest


def _host_step(pipe, probe, nc, variant, case, trace_id, index, step, results, observed, continuations):
    """An ``approve`` or ``cancel`` step: only the fixture's host side issues these."""
    kind = "approve" if "approve" in step else "cancel"
    a = step[kind]
    rid = a["request_id"]
    trace = a.get("trace_id", trace_id)
    pend = pipe.pending_approval(trace, rid)
    out = None
    if kind == "approve" and variant == "V6":
        orig = next(
            (
                s["envelope"]
                for s in case["steps"]
                if "envelope" in s and s["envelope"]["payload"].get("request_id") == rid
            ),
            None,
        )
        if orig is not None:
            probe.readmit = True
            nc.before_step(variant, pipe)
            result, record = pipe.admit_envelope(orig)
            probe.readmit = False
            results.append(result)
            observed.append(record.to_dict())
            return
    try:
        if kind == "approve":
            d = _approval_decision(a, rid, pend)
            if nc is not None:
                nc.approve_any_id(variant, pend, d)
            out = pipe.continue_approval(
                trace, rid, d, _approval_digest(a, pend), a.get("now", "2026-01-01T00:00:00Z")
            )
        else:
            out = pipe.cancel_approval(trace, rid)
    except ContinuationRefused as e:
        reason = e.reason
    after = pipe.pending_approval(trace, rid)
    row = {
        "step": index,
        "kind": kind,
        "request_id": rid,
        "outcome": "refused" if out is None else "resolved",
        "stage": after.state.value if after else None,
        "reason": reason if out is None else None,
        "message": out[0].error.message if out is not None and out[0].error else None,
    }
    if out is not None:
        results.append(out[0])
        observed.append(out[1].to_dict())
    continuations.append(row)


def run_case(case: dict, variant: Optional[str] = None) -> dict:
    """Run one fixture on a fresh pipeline and mock runtime; returns the verdict. ``variant``
    (``"V1"`` to ``"V6"``) applies a negative control and is for the test-only matrix."""
    nc = _negctl(variant)
    pipe = default_pipeline(
        Limits.from_dict(case.get("limits")), case.get("mapping_table", "mock-table")
    )
    pipe.runtime.provenance_overrides = case.get("mock_provenance", {})
    pipe.runtime.data_overrides = case.get("mock_data", {})
    pipe.runtime.approval_expires_at = case.get("mock_approval", {}).get("expires_at")
    probe = _Probe(pipe.runtime, nc, variant)
    pipe.runtime = probe
    name, trace_id = case["case"], case["trace_id"]
    observed: list = []
    results: list = []
    turns: list = []
    selections: list = []
    continuations: list = []
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
    for index, step in enumerate(case["steps"]):
        if nc is not None and ("envelope" in step or "dialect" in step):
            nc.before_step(variant, pipe)
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
        if "restart" in step:
            # The pipeline is dropped and rebuilt over the same runtime: fresh ledger, no pending
            # approvals.
            pipe = default_pipeline(
                Limits.from_dict(case.get("limits")), case.get("mapping_table", "mock-table")
            )
            pipe.runtime = probe
            continue
        if "mock" in step:
            probe.catalog_override = step["mock"].get("catalog_digest")
            continue
        if "approve" in step or "cancel" in step:
            _host_step(pipe, probe, nc, variant, case, trace_id, index, step, results, observed, continuations)
            continue
        if "input" in step and "dialect" not in step:
            # host-only input registration (0.3 cut P3): never reachable from model input
            try:
                pipe.register_input(step["input"])
            except (ValueError, ProtocolError):
                pass
            # `content` is the text the host put in front of the model; the pipeline never reads it.
            if nc is not None and isinstance(step.get("content"), str):
                nc.host_loop(variant, pipe, name, trace_id, [{"data": {"content": step["content"]}}], observed, results)
            continue
        if "envelope" in step:
            result, record = pipe.admit_envelope(step["envelope"], turn=step.get("turn"))
            if nc is not None:
                nc.host_loop(variant, pipe, name, trace_id, [result.to_dict()], observed, results)
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
        if nc is not None:
            nc.host_loop(variant, pipe, name, trace_id, [r.to_dict() for r in out.results], observed, results)
        results.extend(out.results)
        observed.extend(r.to_dict() for r in out.observed)
        turns.append(out.to_dict())
    # Reported only for cases that assert them, so older verdicts keep their shape.
    inputs = pipe.inputs(trace_id) if "inputs" in case["expected"] else []
    exposure = pipe.runtime.seen_exposure if "exposure" in case["expected"] else []
    if nc is not None:
        nc.finish(variant, results, inputs)
    runtime = {
        "decide_calls": pipe.runtime.decide_calls,
        "execute_calls": pipe.runtime.execute_calls,
    }
    injection = None
    if "injection" in case:
        envelopes = sum(1 for s in case["steps"] if "envelope" in s)
        injection = _judge(case["injection"], probe, turns, envelopes)

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
    if "continuations" in expected and continuations != expected["continuations"]:
        problems.append(f"continuations {continuations!r} != {expected['continuations']!r}")
    if "inputs" in expected:
        want = expected["inputs"]
        ok = len(want) == len(inputs) and all(
            all(g.get(k) == v for k, v in w.items()) for w, g in zip(want, inputs)
        )
        if not ok:
            problems.append(f"inputs {inputs!r} != {want!r}")
    if "exposure" in expected and exposure != expected["exposure"]:
        problems.append(f"exposure {exposure!r} != {expected['exposure']!r}")
    if "injection" in case:
        if injection["injected_intents"] == 0:
            problems.append("injection: the injected intent never reached the runtime (vacuous case)")
        if injection["violations"]:
            problems.append(f"injection: {injection['violations']} effect(s) executed for the injected intent")
        if injection["content_derived"]:
            problems.append(f"injection: {injection['content_derived']} intent(s) derived from content")
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
        "injection": injection,
        "continuations": continuations,
        "problems": problems,
        "results": [canonical_result_payload(r) for r in results],
    }


def run_lifecycle_case(case: dict, variant: Optional[str] = None) -> dict:
    """Drive the Crossveil lifecycle directly (no pipeline). For each step, the lifecycle of
    ``request_id`` (created and mapped on first sight) receives ``decision``; the step records the
    state after the call and whether the call was refused."""
    lifecycles: dict = {}
    minted: dict = {}
    steps: list = []
    for step in case["steps"]:
        rid = step["request_id"]
        life = lifecycles.get(rid)
        if life is None:
            life = lifecycles[rid] = Lifecycle(rid)
            life.map()
        try:
            decision = Decision.from_dict(step["decision"])
            if variant is not None:
                _negctl(variant).continuation_any_id(variant, minted, rid, decision)
            life.apply_decision(decision)
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


def load_injection_fixtures(fixtures_dir: str) -> list:
    """The ``injection/NN-*.json`` fixtures (0.3 cut I1), sorted by file name. Absent directory = none."""
    return [
        json.loads(p.read_text(encoding="utf-8"))
        for p in sorted(Path(fixtures_dir, "injection").glob("*.json"))
    ]


def load_approval_fixtures(fixtures_dir: str) -> list:
    """The ``approval/NN-*.json`` fixtures (0.3 cut A2), sorted by file name. Absent directory = none."""
    return [
        json.loads(p.read_text(encoding="utf-8"))
        for p in sorted(Path(fixtures_dir, "approval").glob("*.json"))
    ]


def run_suite(fixtures_dir: str, variant: Optional[str] = None) -> tuple:
    """Run a whole fixtures directory: the NN cases, the injection cases, the lifecycle cases.
    Returns ``(verdicts, lifecycle, digest_errors)``; ``variant`` applies a negative control."""
    verdicts: dict = {}
    cases = [json.loads(p.read_text(encoding="utf-8")) for p in sorted(Path(fixtures_dir).glob("*.json"))]
    for case in cases + load_injection_fixtures(fixtures_dir) + load_approval_fixtures(fixtures_dir):
        verdicts[case["case"]] = run_case(case, variant)
    lifecycle: dict = {}
    for path in sorted(Path(fixtures_dir, "lifecycle").glob("*.json")):
        case = json.loads(path.read_text(encoding="utf-8"))
        lifecycle[case["case"]] = run_lifecycle_case(case, variant)
    return verdicts, lifecycle, check_digest_fixtures(fixtures_dir)


def main(argv: Optional[list] = None) -> int:
    ap = argparse.ArgumentParser(prog="interplane-conformance")
    ap.add_argument("fixtures_dir")
    ap.add_argument("--out")
    ap.add_argument("--dump", help="directory for per-case canonical result payloads")
    ap.add_argument("--variant", help="test-only: apply negative control V1 to V6")
    ap.add_argument("--matrix", help="test-only: write the negative-control matrix here")
    args = ap.parse_args(argv)
    if args.matrix:
        from .conformance_negctl import matrix

        spec = str(Path(args.fixtures_dir, "..", "negative-controls.json"))
        m = matrix(args.fixtures_dir, spec)
        Path(args.matrix).write_text(jcs(m) + "\n", encoding="utf-8")
        print(f"negative-control matrix valid: {'true' if m['valid'] else 'false'}")
        return 0 if m["valid"] else 1
    if not args.out:
        ap.error("--out is required")
    if args.variant:
        from .conformance_negctl import VARIANTS

        if args.variant not in VARIANTS:
            ap.error("unknown variant (V1 to V6)")
        print(f"[NEGCTL_BUILT_IN] NEGATIVE CONTROL {args.variant}: failures below are expected")
    verdicts, lifecycle, digests = run_suite(args.fixtures_dir, args.variant)
    if args.dump:
        d = Path(args.dump)
        d.mkdir(parents=True, exist_ok=True)
        for name, verdict in verdicts.items():
            (d / f"{name}.results.json").write_text(jcs(verdict["results"]) + "\n", encoding="utf-8")
    failed = 0
    for name, v in sorted({**verdicts, **lifecycle}.items()):
        print(f"{'PASS' if v['pass'] else 'FAIL'}  {name}")
        for p in v["problems"]:
            print(f"      {p}")
        failed += not v["pass"]
    for name, err in digests:
        print(f"{'PASS' if err is None else 'FAIL'}  {name}")
        if err:
            print(f"      {err}")
        failed += err is not None
    total = len(verdicts) + len(lifecycle)
    print(f"{total - failed}/{total} passed")
    fields = (
        "pass",
        "observed",
        "turns",
        "runtime",
        "selections",
        "inputs",
        "exposure",
        "injection",
        "continuations",
    )
    out = {k: {f: v[f] for f in fields if f in v} for k, v in verdicts.items()}
    for v in out.values():
        for key in ("selections", "inputs", "exposure", "injection", "continuations"):
            if not v.get(key):
                v.pop(key, None)
    out.update({k: {"pass": v["pass"], "steps": v["steps"]} for k, v in lifecycle.items()})
    Path(args.out).write_text(jcs(out) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
