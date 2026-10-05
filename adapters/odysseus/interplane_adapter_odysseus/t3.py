"""T3 runner (bench/PROTOCOL-0.3.md section 2): the Odysseus subset of the 0.3 corpus through the real
Pipeline and OdysseusAuthority.

    ODYSSEUS_SRC=/path/to/odysseus PYTHONPATH=<interplane>/python:<interplane>/adapters/odysseus \
        <odysseus-venv>/bin/python -m interplane_adapter_odysseus.t3 --out t3-verdicts.json

Subset membership comes from ``conformance/runners/adapter_subset.py`` (the rule is normative text in
spec/CORE.md, "Adapter subsets") and must equal ``t3_subset`` of ``conformance/TRUST-DIGEST.txt``.
Every fixture in the subset runs; one the adapter cannot run is a FAIL, never a skip. Injection
cases use the injection judge of ``interplane.conformance`` (violations, content_derived,
injected_intents). Approval cases are judged per A-ODY: every continuation is refused (fail closed,
"approval continuation unsupported on Odysseus"), nothing executes, and the first decision of each
request equals the fixture's. Exit status is non-zero on any FAIL.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from interplane.conformance import _approval_decision, _approval_digest, _judge
from interplane.core import InputRecord
from interplane.crossveil import ContinuationRefused

from . import _odysseus
from .authority import OdysseusAuthority
from .demo import make_pipeline

REPO = Path(__file__).resolve().parents[3]
UNSUPPORTED = "approval continuation unsupported on Odysseus"


def _load_rule():
    sys.path.insert(0, str(REPO / "conformance" / "runners"))
    import adapter_subset

    return adapter_subset


class _Recorder(OdysseusAuthority):
    """The real adapter, recording every decided request and every executed id for the judge."""

    def __init__(self, *a, **kw) -> None:
        super().__init__(*a, **kw)
        self.requests: list = []  # (request_id, capability, canonical arguments)
        self.executed: set = set()

    def decide(self, req, ctx):
        from interplane.core import jcs

        self.requests.append((req.request_id, req.capability, jcs(req.arguments)))
        return super().decide(req, ctx)

    def execute(self, req, decision, ctx):
        self.executed.add(req.request_id)
        return super().execute(req, decision, ctx)


def _frozen(key: str) -> list:
    for line in (REPO / "conformance" / "TRUST-DIGEST.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith(key + ": "):
            return [x for x in line[len(key) + 2:].split(",") if x]
    return []


def _run_plan(plan: dict) -> dict:
    with tempfile.TemporaryDirectory() as ws:
        Path(ws, "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
        authority = _Recorder(ws, admin=True)
        if not authority.available:
            return {"pass": False, "problems": ["Odysseus not importable: set ODYSSEUS_SRC"]}
        trace, case = plan["trace_id"], plan["case"]
        pipe = make_pipeline(authority)
        observed, turns, continuations = [], [], []
        for index, step in enumerate(plan["steps"]):
            if "host_input" in step:
                try:
                    pipe.register_input(InputRecord.from_dict(step["host_input"]["input"]))
                except (ValueError, KeyError):
                    pass
            elif "restart" in step:
                pipe = make_pipeline(authority)
            elif "envelope" in step:
                _, rec = pipe.admit_envelope(step["envelope"], turn=step.get("turn"))
                observed.append(rec.to_dict())
            elif "approve" in step or "cancel" in step:
                kind = "approve" if "approve" in step else "cancel"
                a = dict(step[kind])
                rid, tr = a["request_id"], a.get("trace_id", trace)
                pend = pipe.pending_approval(tr, rid)
                out, reason = None, None
                try:
                    if kind == "approve":
                        aid = a["approval_id"]
                        a["approval_id"] = aid.get("literal") or "mock-approval-" + aid["minted_for"]
                        out = pipe.continue_approval(
                            tr, rid, _approval_decision(a, rid, pend), _approval_digest(a, pend),
                            a.get("now", "2026-01-01T00:00:00Z"))
                    else:
                        out = pipe.cancel_approval(tr, rid)
                except ContinuationRefused as e:
                    reason = e.reason
                after = pipe.pending_approval(tr, rid)
                continuations.append({
                    "step": index, "kind": kind, "request_id": rid,
                    "outcome": "refused" if out is None else "resolved",
                    "stage": after.state.value if after else None, "reason": reason if out is None else None,
                    "message": out[0].error.message if out is not None and out[0].error else None})
                if out is not None:
                    observed.append(out[1].to_dict())
            else:
                out = pipe.run_turn(step["dialect"], step.get("model", ""), step["input"], trace, step["turn"],
                                    message_id_prefix=f"m-{case}-{step['turn']}")
                observed.extend(r.to_dict() for r in out.observed)
                turns.append(out.to_dict())
        problems: list = []
        row: dict = {"observed": observed, "continuations": continuations, "effects_executed": len(authority.executed)}
        if plan["kind"] == "injection":
            envelopes = sum(1 for s in plan["steps"] if "envelope" in s)
            inj = _judge({"target": plan["target"]}, authority, turns, envelopes)
            row["injection"] = inj
            row["injected_decisions"] = sorted({
                authority.decisions[r[0]].decision for r in authority.requests
                if r[1] == plan["target"]["capability"] and plan["target"]["argument_contains"] in r[2]
                and r[0] in authority.decisions})
            if inj["injected_intents"] == 0:
                problems.append("the injected intent never reached the runtime (vacuous case)")
            if inj["violations"]:
                problems.append(f"{inj['violations']} effect(s) executed for the injected intent")
            if inj["content_derived"]:
                problems.append(f"{inj['content_derived']} intent(s) derived from content")
        else:
            if authority.executed:
                problems.append("an effect executed: " + ", ".join(sorted(authority.executed)))
            for c in continuations:
                if c["outcome"] != "refused":
                    problems.append(f"continuation step {c['step']} resolved ({c['stage']}); Odysseus must refuse")
            first: dict = {}
            for o in observed:
                first.setdefault(o["request_id"], o)
            want_first: dict = {}
            for o in plan["expected"]["observed"]:
                want_first.setdefault(o["request_id"], o)
            for rid, want in want_first.items():
                got = first.get(rid)
                if got is None:
                    problems.append(f"no observed row for {rid}")
                    continue
                for k in ("stage", "decision", "status", "error_code", "decide_invoked", "execute_invoked"):
                    if got.get(k) != want.get(k):
                        problems.append(f"first row of {rid}.{k}: {got.get(k)!r} != {want.get(k)!r}")
            pend_ids = [r for r in authority.decisions.values() if r.decision == "requires_approval"]
            row["unsupported_note"] = bool(pend_ids) and all(
                UNSUPPORTED in ((d.runtime_state or {}).get("values") or []) for d in pend_ids)
            if pend_ids and not row["unsupported_note"]:
                problems.append("requires_approval decision does not state: " + UNSUPPORTED)
        row["pass"] = not problems
        row["problems"] = problems
        return row


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    rule = _load_rule()
    plans = rule.plans("odysseus")
    names = [p["name"] for p in plans]
    problems: list = []
    if names != _frozen("t3_subset"):
        problems.append("computed T3 subset differs from conformance/TRUST-DIGEST.txt t3_subset")
    rows = {}
    for plan in plans:
        try:
            rows[plan["name"]] = _run_plan(plan)
        except Exception as e:  # a fixture the adapter cannot run is a FAIL, never a skip
            rows[plan["name"]] = {"pass": False, "problems": [f"runner could not run it: {e!r}"]}
    failed = [n for n, r in rows.items() if not r["pass"]]
    inj = [r["injection"] for r in rows.values() if "injection" in r]
    ody = os.environ.get(_odysseus.ENV)
    head = None
    if ody:
        try:
            head = subprocess.run(["git", "-C", ody, "rev-parse", "--short", "HEAD"], capture_output=True,
                                  text=True, timeout=10).stdout.strip() or None
        except (OSError, subprocess.SubprocessError):
            pass
    summary = {
        "system": "T3", "odysseus_src": ody, "odysseus_head": head, "cases": len(rows),
        "passed": len(rows) - len(failed), "failed": failed, "subset_problems": problems,
        "violations": sum(i["violations"] for i in inj), "content_derived": sum(i["content_derived"] for i in inj),
        "approval_note": UNSUPPORTED,
    }
    Path(args.out).write_text(json.dumps({"summary": summary, "rows": rows}, indent=2, sort_keys=True) + "\n",
                              encoding="utf-8")
    print(f"T3: {summary['passed']}/{summary['cases']} pass, violations={summary['violations']}, "
          f"content_derived={summary['content_derived']}")
    for n in failed:
        print("FAIL", n, "; ".join(rows[n]["problems"]))
    for p in problems:
        print("FAIL", p)
    return 1 if failed or problems else 0


if __name__ == "__main__":
    sys.exit(main())
