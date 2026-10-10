"""Offline demo: a Motus-shaped host calls tools through INTERPLANE and exports evidence.

    PYTHONPATH=<repo>/python:<repo>/adapters/motus python -m interplane_adapter_motus.demo [out.json]

Uses the LOCAL read-only stand-in authority (not AIEN). Motus itself is optional: if it is not
installed this prints so and drives the fake host interface instead.
"""

from __future__ import annotations

import asyncio
import json
import sys
import tempfile
from pathlib import Path

from .gate import Gate
from .host import build_tools, dispatch
from .local_authority import LABEL, LocalReadOnlyAuthority, build_catalog
from .mapping import mapping_for
from .motus_tool import MotusNotInstalled, installed_version, interplane_tool_class, pin_matches
from .verify import verify_bundle


async def run(out_path: Path, workspace: Path) -> int:
    (workspace / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
    authority = LocalReadOnlyAuthority(str(workspace))
    version = installed_version()
    try:
        interplane_tool_class()
        print(f"Motus: installed {version}" + ("" if pin_matches() else " (NOT the pinned 0.4.3)"))
        print("Motus: importable; this demo drives the host-neutral bridge, which needs no Motus runtime.")
    except MotusNotInstalled:
        print("Motus: not installed; using the fake host interface (nothing is authorized by the adapter).")
    gate = Gate(authority, mapping_for(build_catalog(), authority.runtime_id), motus_installed_version=version)
    gate.register_user_turn("Read notes.txt, then save a copy as copy.txt.")
    tools = build_tools(gate)
    print(f"Authority: {LABEL}")
    print(f"trace_id: {gate.trace_id}")

    async def show(label: str, name: str, args) -> None:
        text = await dispatch(tools, name, args)
        rec = gate.evidence()["records"][-1]
        print(f"{label:42} -> {rec['outcome']:17} executed={rec['executed']!s:5} {text[:60]}")

    await show("authorized read-only call", "read_file", {"path": "notes.txt"})
    await show("denied (path escape)", "read_file", {"path": "../../etc/passwd"})
    await show("malformed arguments", "read_file", "{not json")
    await show("held for approval", "write_note", {"path": "copy.txt", "text": "hello\n"})
    held = gate.evidence()["records"][-1]["request_id"]
    first = gate.approve(held, lambda p: authority.issue_continuation(p, approved=True))
    print(f"{'approval continuation (first)':42} -> {first.outcome:17} executed={first.executed!s:5}")
    second = gate.approve(held, lambda p: authority.issue_continuation(p, approved=True))
    print(f"{'approval continuation (second, refused)':42} -> {second.outcome:17} executed={second.executed!s:5} {second.record['error_code']}")
    dup = gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="mc-000001")
    print(f"{'duplicate request id replay':42} -> {dup.outcome:17} executed={dup.executed!s:5} {dup.record['error_code']}")
    bundle = gate.evidence()
    out_path.write_text(json.dumps(bundle, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    problems = verify_bundle(json.loads(out_path.read_text(encoding="utf-8")))
    print(f"evidence: {out_path} ({len(bundle['records'])} records) verify={'OK' if not problems else problems}")
    return 0 if not problems else 1


def main(argv: list | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    with tempfile.TemporaryDirectory(prefix="interplane-motus-demo-") as tmp:
        out = Path(argv[0]) if argv else Path(tmp) / "bundle.json"
        ws = Path(tmp) / "ws"
        ws.mkdir()
        rc = asyncio.run(run(out, ws))
        if not argv:
            print("(bundle written to a temporary path; pass a path argument to keep it)")
        return rc


if __name__ == "__main__":
    sys.exit(main())
