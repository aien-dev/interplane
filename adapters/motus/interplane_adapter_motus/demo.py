"""Offline demo: a Motus-shaped host calls tools through INTERPLANE and exports evidence.

    PYTHONPATH=<repo>/python:<repo>/adapters/motus python -m interplane_adapter_motus.demo [out.json]

Default: the LOCAL read-only stand-in authority (not AIEN). With ``--aien <workspace>`` the same
scripted calls go to the real AIEN authority through ``adapters/aien``'s ``aien-authority-bridge``
(binary path from the INTERPLANE_AIEN_BRIDGE environment variable). Motus itself is optional: if it
is not installed this prints so and drives the fake host interface instead.
"""

from __future__ import annotations

import asyncio
import json
import sys
import tempfile
from pathlib import Path

from .aien_bridge import ENV_BRIDGE, AienBridgeAuthority
from .gate import Gate
from .host import build_tools, dispatch
from .local_authority import LABEL, LocalReadOnlyAuthority, build_catalog
from .mapping import mapping_for
from .motus_tool import MotusNotInstalled, installed_version, interplane_tool_class, pin_matches
from .verify import verify_bundle


AIEN_LABEL = "AIEN (adapters/aien via bridge)"


async def run(out_path: Path, workspace: Path, aien: bool = False) -> int:
    (workspace / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
    if aien:
        authority = AienBridgeAuthority(str(workspace))
        if not authority.available:
            print(f"Authority: {AIEN_LABEL} NOT AVAILABLE ({authority.fault}); set {ENV_BRIDGE} to the built bridge.")
            return 2
        table, hold_cap = authority.mapping_table(), "write_file"
        hold_args = {"path": "copy.txt", "content": "hello\n"}
        label = AIEN_LABEL
    else:
        authority = LocalReadOnlyAuthority(str(workspace))
        table, hold_cap = mapping_for(build_catalog(), authority.runtime_id), "write_note"
        hold_args = {"path": "copy.txt", "text": "hello\n"}
        label = LABEL
    version = installed_version()
    try:
        interplane_tool_class()
        print(f"Motus: installed {version}" + ("" if pin_matches() else " (NOT the pinned 0.4.3)"))
        print("Motus: importable; this demo drives the host-neutral bridge, which needs no Motus runtime.")
    except MotusNotInstalled:
        print("Motus: not installed; using the fake host interface (nothing is authorized by the adapter).")
    gate = Gate(authority, table, motus_installed_version=version)
    gate.register_user_turn("Read notes.txt, then save a copy as copy.txt.")
    tools = build_tools(gate)
    print(f"Authority: {label}")
    print(f"trace_id: {gate.trace_id}")

    async def show(label: str, name: str, args) -> None:
        text = await dispatch(tools, name, args)
        rec = gate.evidence()["records"][-1]
        print(f"{label:42} -> {rec['outcome']:17} executed={rec['executed']!s:5} {text[:60]}")

    await show("authorized read-only call", "read_file", {"path": "notes.txt"})
    await show("denied (path escape)", "read_file", {"path": "../../etc/passwd"})
    await show("malformed arguments", "read_file", "{not json")
    await show("held for approval", hold_cap, hold_args)
    held = gate.evidence()["records"][-1]["request_id"]
    first = gate.approve(held, lambda p: authority.issue_continuation(p, approved=True))
    print(f"{'approval continuation (first)':42} -> {first.outcome:17} executed={first.executed!s:5}")
    second = gate.approve(held, lambda p: authority.issue_continuation(p, approved=True))
    print(f"{'approval continuation (second, refused)':42} -> {second.outcome:17} executed={second.executed!s:5} {second.record['error_code']}")
    dup = gate.call("read_file", json.dumps({"path": "notes.txt"}), call_id="mc-000001")
    print(f"{'duplicate request id replay':42} -> {dup.outcome:17} executed={dup.executed!s:5} {dup.record['error_code']}")
    bundle = gate.evidence()
    authority_close = getattr(authority, "close", None)
    if authority_close:
        authority_close()
    out_path.write_text(json.dumps(bundle, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    problems = verify_bundle(json.loads(out_path.read_text(encoding="utf-8")))
    print(f"evidence: {out_path} ({len(bundle['records'])} records) verify={'OK' if not problems else problems}")
    return 0 if not problems else 1


def main(argv: list | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    aien_ws = None
    if "--aien" in argv:
        i = argv.index("--aien")
        if i + 1 >= len(argv):
            print("usage: demo [--aien <workspace>] [out.json]")
            return 2
        aien_ws = Path(argv[i + 1])
        del argv[i : i + 2]
    with tempfile.TemporaryDirectory(prefix="interplane-motus-demo-") as tmp:
        out = Path(argv[0]) if argv else Path(tmp) / "bundle.json"
        ws = aien_ws or Path(tmp) / "ws"
        ws.mkdir(parents=True, exist_ok=True)
        rc = asyncio.run(run(out, ws, aien=aien_ws is not None))
        if not argv:
            print("(bundle written to a temporary path; pass a path argument to keep it)")
        return rc


if __name__ == "__main__":
    sys.exit(main())
