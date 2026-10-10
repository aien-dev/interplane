"""Independent verifier for an evidence bundle.

    python -m interplane_adapter_motus.verify bundle.json

Recomputes every record digest and the bundle digest and checks the cross-field rules. It needs
only the stdlib-only ``interplane`` Core package (for canonical JSON); it does not import the
gate, Motus or any authority. Exit status 0 means the file is internally consistent and untampered
since export. It does not prove the authority was honest; the digests bind what was recorded.
"""

from __future__ import annotations

import json
import sys


from .evidence import SCHEMA, bundle_digest, record_digest
from .ids import valid_id, valid_trace_id

REQUIRED = (
    "kind", "trace_id", "request_id", "seq", "decision", "executed", "outcome",
    "effect_certainty", "retried", "motus", "record_digest",
)


def _no_constant(name: str):
    raise ValueError(f"non-finite number {name}")


def verify_bundle(bundle: object) -> list:
    """Return a list of problems; empty means the bundle verifies."""
    if not isinstance(bundle, dict):
        return ["bundle is not an object"]
    problems: list = []
    if bundle.get("schema") != SCHEMA:
        problems.append(f"unknown schema: {bundle.get('schema')!r}")
    trace_id = bundle.get("trace_id")
    if not valid_trace_id(trace_id):
        problems.append("trace_id is not 32 lowercase hex characters")
    header, records = bundle.get("header"), bundle.get("records")
    if not isinstance(header, dict) or not isinstance(records, list):
        return problems + ["header or records missing"]
    digests: list = []
    held: set = set()
    executed_ids: set = set()
    for index, rec in enumerate(records, start=1):
        label = f"record {index}"
        if not isinstance(rec, dict):
            problems.append(f"{label} is not an object")
            continue
        for key in REQUIRED:
            if key not in rec:
                problems.append(f"{label} lacks {key}")
        if rec.get("seq") != index:
            problems.append(f"{label} has seq {rec.get('seq')!r}, expected {index}")
        if rec.get("trace_id") != trace_id:
            problems.append(f"{label} carries a different trace_id")
        rid = rec.get("request_id")
        if rid is not None and not valid_id(rid):
            problems.append(f"{label} request_id is invalid")
        try:
            expected = record_digest(rec)
        except Exception:  # noqa: BLE001
            expected = None
        if rec.get("record_digest") != expected:
            problems.append(f"{label} digest mismatch (record was changed)")
        digests.append(rec.get("record_digest"))
        if rec.get("retried") is not False:
            problems.append(f"{label} claims a retry; the adapter never retries")
        kind, executed = rec.get("kind"), rec.get("executed")
        if executed is True:
            if rec.get("decision") != "authorized":
                problems.append(f"{label} executed without an authorized decision")
            if rid in executed_ids:
                problems.append(f"{label} executes request {rid} a second time")
            executed_ids.add(rid)
            if kind == "continuation" and rid not in held:
                problems.append(f"{label} continues a request that was never held for approval")
            if rec.get("capability_request_digest") is None:
                problems.append(f"{label} executed with no capability request digest")
        if kind == "call" and rec.get("outcome") == "approval_pending" and executed is False:
            held.add(rid)
        if rec.get("outcome") in ("timeout", "uncertain", "cancelled") and rec.get("retried"):
            problems.append(f"{label} retried after {rec['outcome']}")
    if digests and not all(isinstance(d, str) for d in digests):
        problems.append("a record digest is missing")
    else:
        recomputed = bundle_digest(bundle.get("schema", ""), trace_id, header, digests)
        if bundle.get("bundle_digest") != recomputed:
            problems.append("bundle digest mismatch")
    return problems


def main(argv: list | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print("usage: python -m interplane_adapter_motus.verify bundle.json", file=sys.stderr)
        return 2
    try:
        with open(argv[0], encoding="utf-8") as fh:
            bundle = json.load(fh, parse_constant=_no_constant)
    except (OSError, ValueError) as err:
        print(f"FAIL: cannot read bundle: {err}")
        return 1
    problems = verify_bundle(bundle)
    if problems:
        print("FAIL: bundle does not verify")
        for p in problems:
            print(f"  - {p}")
        return 1
    n = len(bundle["records"])
    print(f"OK: {n} record(s), trace {bundle['trace_id']}, bundle digest {bundle['bundle_digest']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
