"""Deterministic helpers for the 0.2y authoring steps (stdlib only; no model).

PREREG-0.2y section 5 / heldout-0.2y/AUTHORING.md section 4:
  * facts are handed to the author without their file (``facts_for_author``);
  * brief E (everyday) and brief Q (question) draw from disjoint fact subsets, split by fact id
    (odd serial -> E, even serial -> Q), and the split is recorded (``split_record``);
  * the fact pool must cover the base requests plus the maximum top-ups for each brief
    (``pool_check``); when it does not, the fixture step is repeated for the number of extra workspaces
    the check computes, never by hand.
"""

from __future__ import annotations

import math
import re

BRIEFS = ("everyday", "question")
FIRST_PASS = 36
TOP_UP_STEP = 12
MAX_TOP_UPS = 3
FACTS_PER_WORKSPACE = 40  # minimum asked for in the fixture brief


def per_brief_need() -> int:
    """Base requests plus the maximum top-ups, for one brief: 36 + 12 + 24 + 36 = 108."""
    return FIRST_PASS + sum(TOP_UP_STEP * r for r in range(1, MAX_TOP_UPS + 1))


def workspaces_needed(per_ws: int = FACTS_PER_WORKSPACE) -> int:
    """Workspaces the first fixture call asks for: ceil(both briefs' need / facts per workspace) = 6."""
    return math.ceil(len(BRIEFS) * per_brief_need() / per_ws)


def with_ids(workspaces: list) -> list:
    """[{"slug", "facts": [{"fact": ..., "file": ...}]}] -> [{"id", "slug", "serial", "fact"}], in author order.
    The id is ``<slug>-<serial:03d>``; the serial counts from 1 inside a workspace."""
    out = []
    for ws in workspaces:
        for n, f in enumerate(ws["facts"], 1):
            out.append({"id": f"{ws['slug']}-{n:03d}", "slug": ws["slug"], "serial": n, "fact": f["fact"]})
    return out


def brief_of(fact: dict) -> str:
    """Odd serial -> everyday, even serial -> question. A pure function of the fact id."""
    return BRIEFS[0] if fact["serial"] % 2 == 1 else BRIEFS[1]


def split_record(workspaces: list) -> dict:
    facts = with_ids(workspaces)
    return {b: [f["id"] for f in facts if brief_of(f) == b] for b in BRIEFS}


def facts_for_author(workspaces: list, brief: str, used: set = frozenset()) -> list:
    """What one brief may see: [{"id", "fact"}] for its own subset, minus the ids already used. No file, no path."""
    return [{"id": f["id"], "fact": f["fact"]} for f in with_ids(workspaces) if brief_of(f) == brief and f["id"] not in used]


PATH_LIKE = re.compile(r"[/\\]|\.[a-z0-9]{1,5}(?![a-z0-9])")


def fact_errors(workspaces: list, extension_pattern: str = None) -> list:
    """A fact must not carry a path, a file name or an extension (it is shown to the author, who must not
    be able to copy a file name into a request). Returns the offending fact ids."""
    errs = []
    for ws in workspaces:
        names = {p.rsplit("/", 1)[-1].lower() for p in ws.get("files", {})}
        for n, f in enumerate(ws["facts"], 1):
            text = f["fact"].lower()
            fid = f"{ws['slug']}-{n:03d}"
            if re.search(r"[/\\]", text) or any(re.search(r"(?<![a-z0-9_])" + re.escape(x) + r"(?![a-z0-9_])", text) for x in names):
                errs.append(f"{fid}: fact contains a path or a file name")
            elif extension_pattern and re.search(extension_pattern, text):
                errs.append(f"{fid}: fact contains a file extension")
    return errs


def pool_check(workspaces: list) -> dict:
    """Is the pool large enough? ``extra_workspaces`` is the deterministic enlargement when it is not."""
    split = split_record(workspaces)
    need = per_brief_need()
    short = max(max(0, need - len(split[b])) for b in BRIEFS)
    extra = math.ceil(short / (FACTS_PER_WORKSPACE // 2)) if short else 0
    return {"ok": short == 0, "need_per_brief": need, "available": {b: len(split[b]) for b in BRIEFS},
            "shortfall_per_brief": {b: max(0, need - len(split[b])) for b in BRIEFS}, "extra_workspaces": extra}
