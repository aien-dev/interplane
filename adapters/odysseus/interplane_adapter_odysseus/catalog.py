"""The recorded Odysseus catalog (extracted from a live import at commit a8c147b)."""

from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Optional

from interplane.core import Catalog

RECORD_NAME = "catalog.odysseus-a8c147b.json"
RECORD_PATH = Path(__file__).resolve().parent.parent / RECORD_NAME
RUNTIME_ID = "odysseus"


def load_record(path: Optional[Path] = None) -> dict:
    """The recorded catalog as plain JSON (a fresh copy; the file is the record, never edited)."""
    return json.loads(Path(path or RECORD_PATH).read_text(encoding="utf-8"))


def load_catalog(path: Optional[Path] = None) -> Catalog:
    """The recorded catalog as a Core ``Catalog`` (domains empty, no canonical aliases)."""
    return Catalog.from_dict(load_record(path))


def tool_names(path: Optional[Path] = None) -> list:
    return [c["name"] for c in load_record(path)["capabilities"]]


def effects_of(name: str, record: Optional[dict] = None) -> list:
    """The recorded ``odysseus.tool_effect`` values for one tool."""
    for cap in (record or load_record())["capabilities"]:
        if cap["name"] == name:
            return list((cap.get("runtime_effects") or {}).get("values") or [])
    raise KeyError(name)


def clone(record: dict) -> dict:
    return copy.deepcopy(record)
