"""Re-extract the recorded Odysseus catalog from a live import (no hand edits).

    ODYSSEUS_SRC=<checkout> PYTHONPATH=../../python:. <odysseus-venv>/bin/python \
        tools/extract_catalog.py --commit <short> --extracted YYYY-MM-DD --out catalog.odysseus-<short>.json

Source: ``src/tool_schemas.FUNCTION_TOOL_SCHEMAS`` (name, description, parameters) and
``src/tool_capabilities.capabilities_for_action(name, "")`` (``odysseus.tool_effect`` values).
Capabilities are sorted by name; ``schema_digest`` and ``catalog_digest`` come from Core
(``digest(parameters)`` and ``Catalog.computed_digest``). Effects are the UNION over every known action of an action-dependent tool (see ``_effects``).

Control: to reproduce the OLD record ``catalog.odysseus-2992bf6.json`` byte for byte, run it against a
2992bf6 checkout with ``--empty-action`` (the old one-call-with-no-action method) and
``--formula "interplane Catalog.computed_digest: sha256 over JCS of sorted [name, sha256(JCS(parameters))] pairs (recomputed 2026-10-04; the original extraction used an ad-hoc formula)"``
and ``--extracted 2026-10-04 --commit 2992bf6``.
"""

from __future__ import annotations

import argparse
import json
import subprocess

from interplane.core import Catalog, digest
from interplane_adapter_odysseus import _odysseus

FORMULA = ("interplane Catalog.computed_digest: sha256 over JCS of sorted [name, sha256(JCS(parameters))] "
           "pairs")


def _effects(od, name: str, empty_action: bool) -> list:
    """``odysseus.tool_effect`` values to record for one tool.

    Security metadata must never be weaker than upstream, so for a tool whose effects depend on the
    action we record the union over all its known actions, plus the empty-action fallback and the
    tool-level base: every action named in ``_PRIVATE_ACTION_READS``, ``_PRIVATE_ACTION_WRITES`` and
    ``_ACTION_DESTRUCTIVE`` (tool_capabilities.py:332, :352, :403 at a8c147b; the tables are found
    generically, not by tool name). ``empty_action`` reproduces the old single-call record.
    """
    tc = od.tool_capabilities
    found = set(e.value for e in tc.capabilities_for_action(name, "").effects)
    if empty_action:
        return sorted(found)
    found |= set(e.value for e in tc.capabilities_for_tool(name).effects)
    actions = set()
    for table in ("_PRIVATE_ACTION_READS", "_PRIVATE_ACTION_WRITES", "_ACTION_DESTRUCTIVE"):
        actions |= set(getattr(tc, table, {}).get(name, ()))
    for action in sorted(actions):
        found |= set(e.value for e in tc.capabilities_for_action(name, {"action": action}).effects)
    return sorted(found)


def extract(commit: str, extracted: str, formula: str = FORMULA, empty_action: bool = False) -> dict:
    od = _odysseus.load()
    if od is None:
        raise SystemExit("set ODYSSEUS_SRC to an Odysseus checkout")
    head = subprocess.run(["git", "-C", od.root, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if not head.startswith(commit):
        raise SystemExit(f"checkout HEAD {head[:12]} is not {commit}")
    caps = []
    for schema in od.tool_schemas.FUNCTION_TOOL_SCHEMAS:
        fn = schema["function"]
        known = od.tool_capabilities.capabilities_for_action(fn["name"], "").known
        caps.append({
            "canonical": None,
            "description": fn["description"],
            "domains": [],
            "name": fn["name"],
            "parameters": fn["parameters"],
            "runtime_effects": {
                "known": bool(known),
                "values": _effects(od, fn["name"], empty_action),
                "vocabulary": "odysseus.tool_effect",
            },
            "schema_digest": digest(fn["parameters"]),
        })
    caps.sort(key=lambda c: c["name"])
    rec = {
        "capabilities": caps,
        "catalog_version": commit,
        "extensions": {
            "extracted": extracted,
            "source": ("src/tool_schemas.py FUNCTION_TOOL_SCHEMAS + src/tool_capabilities.capabilities_for_action "
                       f"at odysseus-dev/odysseus@{commit}"),
            "digest_formula": formula,
        },
        "kind": "catalog",
        "runtime": "odysseus",
    }
    rec["catalog_digest"] = Catalog.from_dict(rec).computed_digest()
    return rec


def _dump(rec: dict) -> str:
    """Sorted keys everywhere except ``extensions``, which keeps extracted, source, digest_formula order."""
    ext = rec.pop("extensions")
    text = json.dumps(rec, indent=2, sort_keys=True, ensure_ascii=False)
    ext_text = json.dumps(ext, indent=2, ensure_ascii=False).replace("\n", "\n  ")
    marker = '  "kind": "catalog"'
    assert text.count(marker) == 1
    rec["extensions"] = ext
    return text.replace(marker, f'  "extensions": {ext_text},\n' + marker)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--commit", required=True)
    ap.add_argument("--extracted", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--empty-action", action="store_true", help="old method: effects of one call with no action (control only)")
    ap.add_argument("--formula", default=FORMULA, help="digest_formula note (only to reproduce an older record)")
    a = ap.parse_args()
    rec = extract(a.commit, a.extracted, a.formula, a.empty_action)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(_dump(rec) + "\n")
    print(f"{len(rec['capabilities'])} capabilities, {rec['catalog_digest']}")


if __name__ == "__main__":
    main()
