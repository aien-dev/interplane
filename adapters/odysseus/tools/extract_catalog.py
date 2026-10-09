"""Re-extract the recorded Odysseus catalog from a live import (no hand edits).

    ODYSSEUS_SRC=<checkout> PYTHONPATH=../../python:. <odysseus-venv>/bin/python \
        tools/extract_catalog.py --commit <short> --extracted YYYY-MM-DD --out catalog.odysseus-<short>.json

Source: ``src/tool_schemas.FUNCTION_TOOL_SCHEMAS`` (name, description, parameters) and
``src/tool_capabilities.capabilities_for_action(name, "")`` (``odysseus.tool_effect`` values).
Capabilities are sorted by name; ``schema_digest`` and ``catalog_digest`` come from Core
(``digest(parameters)`` and ``Catalog.computed_digest``). Validation: run against 2992bf6 and the
output is byte-identical to ``catalog.odysseus-2992bf6.json``.
"""

from __future__ import annotations

import argparse
import json
import subprocess

from interplane.core import Catalog, digest
from interplane_adapter_odysseus import _odysseus

FORMULA = ("interplane Catalog.computed_digest: sha256 over JCS of sorted [name, sha256(JCS(parameters))] "
           "pairs")


def extract(commit: str, extracted: str, formula: str = FORMULA) -> dict:
    od = _odysseus.load()
    if od is None:
        raise SystemExit("set ODYSSEUS_SRC to an Odysseus checkout")
    head = subprocess.run(["git", "-C", od.root, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if not head.startswith(commit):
        raise SystemExit(f"checkout HEAD {head[:12]} is not {commit}")
    caps = []
    for schema in od.tool_schemas.FUNCTION_TOOL_SCHEMAS:
        fn = schema["function"]
        effects = od.tool_capabilities.capabilities_for_action(fn["name"], "")
        caps.append({
            "canonical": None,
            "description": fn["description"],
            "domains": [],
            "name": fn["name"],
            "parameters": fn["parameters"],
            "runtime_effects": {
                "known": bool(effects.known),
                "values": sorted(e.value for e in effects.effects),
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
    ap.add_argument("--formula", default=FORMULA, help="digest_formula note (only to reproduce an older record)")
    a = ap.parse_args()
    rec = extract(a.commit, a.extracted, a.formula)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(_dump(rec) + "\n")
    print(f"{len(rec['capabilities'])} capabilities, {rec['catalog_digest']}")


if __name__ == "__main__":
    main()
