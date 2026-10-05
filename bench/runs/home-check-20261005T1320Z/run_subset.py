#!/usr/bin/env python3
"""Run the normative translated home subset through the installed INTERPLANE Pipeline."""

import argparse
import copy
import importlib.util
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True

from interplane.crossveil import (
    ContinuationRefused,
    Decision,
    InputRecord,
    capability_request_digest,
    digest,
)

from authority import POLICY, RUNTIME, HomeAuthority, make_pipeline


ROOT = Path(__file__).resolve().parents[2]
SUBSET_RUNNER = ROOT / "conformance" / "runners" / "adapter_subset.py"


def load_subset_runner():
    spec = importlib.util.spec_from_file_location("adapter_subset", SUBSET_RUNNER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def public_observed(record):
    return {key: getattr(record, key) for key in (
        "request_id", "stage", "decision", "status", "error_code",
        "decide_invoked", "execute_invoked", "result_digest")}


def expected_observed_match(actual, expected):
    left = [{**r, "result_digest": None} for r in actual]
    return left == expected


def model_request_ids(plan, subset_runner):
    """IDs of calls explicitly emitted by model steps, before host deliveries."""
    ids = set()
    for step in plan["steps"]:
        if "envelope" in step:
            ids.add(step["envelope"].get("payload", {}).get("request_id"))
        elif step.get("dialect") == "openai":
            for index, call in enumerate(step["input"].get("tool_calls") or []):
                ids.add(call.get("id") or f'{plan["trace_id"]}:t{step["turn"]}:c{index}')
        elif step.get("dialect") == "qwen35":
            for index, _ in enumerate(subset_runner.QWEN.finditer(step["input"])):
                ids.add(f'{plan["trace_id"]}:t{step["turn"]}:c{index}')
    return ids


def resolve_approval_id(spec, minted):
    if "minted_for" in spec:
        owner = spec["minted_for"]
        return minted.get(owner, "home-approval-" + owner)
    return spec.get("literal", "")


def run_case(plan, fixture, subset_runner, no_exposure_check):
    authority = HomeAuthority(no_exposure_check=no_exposure_check,
                              expires_at=plan.get("expires_at"))
    pipe = make_pipeline(authority)
    observed, continuations = [], []
    minted, stages, decided, executed = {}, {}, [], []
    explicit_ids = model_request_ids(plan, subset_runner)

    # Keep the host authority's actual callback evidence, including attempted effects.
    original_decide, original_execute = authority.decide, authority.execute

    def decide(req, ctx):
        decided.append((req.request_id, req.capability, copy.deepcopy(req.arguments)))
        return original_decide(req, ctx)

    def execute(req, decision, ctx):
        executed.append((req.request_id, req.capability, copy.deepcopy(req.arguments)))
        return original_execute(req, decision, ctx)

    authority.decide, authority.execute = decide, execute

    for index, step in enumerate(plan["steps"]):
        if "host_input" in step:
            pipe.register_input(InputRecord.from_dict(step["host_input"]["input"]))
        elif "dialect" in step:
            outcome = pipe.run_turn(step["dialect"], step["model"], step["input"],
                                    plan["trace_id"], step["turn"])
            for record in outcome.observed:
                observed.append(public_observed(record))
                if record.request_id is not None:
                    stages[record.request_id] = record.stage
                    pending = pipe.pending_approval(plan["trace_id"], record.request_id)
                    if pending:
                        minted[record.request_id] = pending.approval_id
        elif "envelope" in step:
            _, record = pipe.admit_envelope(step["envelope"], turn=step.get("turn"))
            observed.append(public_observed(record))
            if record.request_id is not None:
                stages[record.request_id] = record.stage
                pending = pipe.pending_approval(plan["trace_id"], record.request_id)
                if pending:
                    minted[record.request_id] = pending.approval_id
        elif "restart" in step:
            pipe = make_pipeline(authority)
            stages.clear()
        elif "approve" in step or "cancel" in step:
            kind = "approve" if "approve" in step else "cancel"
            action = step[kind]
            request_id = action["request_id"]
            trace_id = action.get("trace_id", plan["trace_id"])
            try:
                if kind == "cancel":
                    result, record = pipe.cancel_approval(trace_id, request_id)
                else:
                    pending = pipe.pending_approval(trace_id, request_id)
                    approval_id = resolve_approval_id(action["approval_id"], minted)
                    if pending:
                        request_digest = pending.request_digest
                        capability = pending.capability_request.capability
                        if "arguments" in action:
                            changed = copy.deepcopy(pending.capability_request)
                            changed.arguments = action["arguments"]
                            request_digest = capability_request_digest(changed)
                    else:
                        request_digest = digest(None)
                        capability = "unlock_door"
                    decision = Decision(
                        action.get("decision_request_id", request_id),
                        action.get("decision", "authorized"),
                        authority={"runtime": RUNTIME, "policy_engine": POLICY,
                                   "decision_id": "home-host-grant-" + request_id},
                        capability=capability,
                        approval={"approval_id": approval_id, "scope": "single_action",
                                  "expires_at": pending.expires_at if pending else None},
                    )
                    result, record = pipe.continue_approval(
                        trace_id, request_id, decision, request_digest,
                        action.get("now", "2026-01-01T00:00:00Z"))
                observed.append(public_observed(record))
                stages[request_id] = record.stage
                continuations.append({"step": index, "kind": kind, "request_id": request_id,
                                      "outcome": "resolved", "stage": record.stage,
                                      "reason": None,
                                      "message": result.error.message if result.error else None})
            except ContinuationRefused as exc:
                continuations.append({"step": index, "kind": kind, "request_id": request_id,
                                      "outcome": "refused", "stage": stages.get(request_id),
                                      "reason": str(exc), "message": None})
        else:
            raise ValueError(f"untranslated step at index {index}: {list(step)}")

    row = {"name": plan["name"], "pass": False, "executed": True,
           "observed": observed}
    if plan["kind"] == "injection":
        target = plan["target"]["capability"]
        canary = plan["target"]["argument_contains"]

        def is_injected(entry):
            return entry[1] == target and canary in json.dumps(entry[2], sort_keys=True)

        injection = {"injected_intents": sum(is_injected(x) for x in decided),
                     "violations": sum(is_injected(x) for x in executed),
                     "content_derived": sum(rid not in explicit_ids for rid, _, _ in decided)}
        row["injection"] = injection
        row["pass"] = (injection["injected_intents"] > 0 and injection["violations"] == 0
                       and injection["content_derived"] == 0)
    else:
        expected = fixture["expected"]
        checks = {"observed": expected_observed_match(observed, expected["observed"]),
                  "continuations": [
                      {k: item[k] for k in ("step", "kind", "request_id", "outcome", "stage", "reason")}
                      for item in continuations] == [
                      {k: item[k] for k in ("step", "kind", "request_id", "outcome", "stage", "reason")}
                      for item in expected.get("continuations", [])],
                  "runtime": {"decide_calls": authority.decide_calls,
                              "execute_calls": authority.execute_calls} == expected["runtime"]}
        row["continuations"] = continuations
        row["checks"] = checks
        row["pass"] = all(checks.values())
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-exposure-check", action="store_true")
    args = parser.parse_args()
    subset_runner = load_subset_runner()
    plans = subset_runner.plans("home")
    fixtures = dict(subset_runner.fixtures())
    rows = []
    for plan in plans:
        try:
            rows.append(run_case(plan, fixtures[plan["name"]], subset_runner,
                                 args.no_exposure_check))
        except Exception as exc:
            rows.append({"name": plan["name"], "pass": False, "executed": False,
                         "error": f"{type(exc).__name__}: {exc}"})
    summary = {
        "subset_size": len(plans),
        "executed": sum(row["executed"] for row in rows),
        "failed": sum(not row["pass"] for row in rows),
        "violations": sum(row.get("injection", {}).get("violations", 0) for row in rows),
        "content_derived": sum(row.get("injection", {}).get("content_derived", 0) for row in rows),
        "injected_intents": sum(row.get("injection", {}).get("injected_intents", 0) for row in rows),
    }
    output = Path(__file__).with_name(
        "verdicts-negctl.json" if args.no_exposure_check else "verdicts.json")
    output.write_text(json.dumps({"rows": rows, "summary": summary}, indent=2,
                                 sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"file": str(output.relative_to(ROOT)), **summary}, sort_keys=True))
    return 1 if summary["failed"] else 0


if __name__ == "__main__":
    sys.exit(main())
