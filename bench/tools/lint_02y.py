"""Leakage lint and acceptance replay for the INTERPLANE 0.2y corpus (stdlib only; no model, no GPU).

PREREG-0.2y section 5, implemented exactly. One request in, a list of rejection codes out:

  LEAK_TOOL_NAME   a full catalog capability name occurs as a word
  LEAK_TOOL_TOKEN  a token of a capability name (split on _ . -, length >= 3) occurs as a word and
                   is not itself a domains.json keyword (the keyword exemption, section 5)
  BANNED_WORD      file, path, grep, search tool, tool, function, command (plural -s allowed too);
                   banned even where they are keywords
  EXTENSION        a domains.json filename_pattern extension, or .rst .adoc .org .tsv .conf .xml .html
  PATH_SEPARATOR   "/" or "\\"
  FIXTURE_NAME     a file name that equals a file name of the task's fixture
  DOMAIN_FILE_KEYWORD  the request derives the filesystem domain through a keyword or a file name
                   (the default list is the only allowed way to land in the filesystem domain)
  DOMAIN_MULTI     the request derives more than one domain
  DOMAIN_FILE_TOOLS  (clarification, reported as a deviation) the single non-file domain it derives
                   also exposes a workspace-read tool (the code domain), so the selector would not be wrong

Group (``group`` of a passing request): "nonfile" when it derives exactly one non-file domain,
"default" when it derives the default list (no keyword, no file name), otherwise None (rejected).

Authors see only ``AUTHOR_MESSAGES`` (generic text), never a word list or the offending token; the
token goes to ``detail`` in the authoring log.

``replay`` re-derives every accept / discard decision of an authoring log in request-id order and
checks the 24 / 24 group rule, the top-up rule and the recorded lint results.
"""

from __future__ import annotations

import re
from typing import Iterable

ALWAYS_BANNED = ("file", "path", "grep", "search tool", "tool", "function", "command")
EXTRA_EXTENSIONS = (".rst", ".adoc", ".org", ".tsv", ".conf", ".xml", ".html")
BASE_TOOLS = ("ask_user", "ls", "glob", "grep", "read_file")
WORKSPACE_READ_TOOLS = ("ls", "glob", "grep", "read_file")
GROUP_SIZE = 24
MAX_TOP_UPS = 3
FIRST_PASS_PER_BRIEF = 36

AUTHOR_MESSAGES = {
    "LEAK_TOOL_NAME": "The request names a tool.",
    "LEAK_TOOL_TOKEN": "The request uses a word that belongs to a tool name.",
    "BANNED_WORD": "The request uses a word that is not allowed.",
    "EXTENSION": "The request contains a file extension.",
    "PATH_SEPARATOR": "The request contains a path separator.",
    "FIXTURE_NAME": "The request contains the name of a file in the workspace.",
    "DOMAIN_FILE_KEYWORD": "The request matches the file area of the selection rule.",
    "DOMAIN_MULTI": "The request matches more than one area of the selection rule.",
    "DOMAIN_FILE_TOOLS": "The request matches an area that already offers workspace reading.",
}


def _word(text: str, word: str) -> bool:
    """``word`` occurs in lowercase ``text`` with no letter, digit or underscore on either side
    (the boundary rule of domains.json)."""
    return re.search(r"(?<![a-z0-9_])" + re.escape(word) + r"(?![a-z0-9_])", text) is not None


def all_keywords(rule: dict) -> set:
    return {w for words in rule["keywords"].values() for w in words}


def filename_extensions(rule: dict) -> list:
    """The extensions inside domains.json filename_pattern, plus the section 5 additions."""
    m = re.search(r"\\\.\(([^)]*)\)", rule["filename_pattern"])
    if not m:
        raise ValueError("filename_pattern has no extension group")
    return ["." + e for e in m.group(1).split("|")] + list(EXTRA_EXTENSIONS)


def name_tokens(names: Iterable) -> set:
    return {t for n in names for t in re.split(r"[_.\-]", n.lower()) if len(t) >= 3}


def derive_match(request: str, rule: dict) -> dict:
    """domain_match derivation (bench/domains.json applied to the request), with the evidence:
    ``matched`` maps each matched domain to its keywords, ``file_name`` is the matching file name
    or None, ``default_used`` is True when nothing matched."""
    text = request.lower()
    matched = {}
    for domain, words in rule["keywords"].items():
        hits = [w for w in words if _word(text, w)]
        if hits:
            matched[domain] = hits
    fn = re.search(r"(?<![a-z0-9_])" + rule["filename_pattern"], text)
    if fn:
        matched.setdefault("filesystem", [])
    domains = sorted(matched) if matched else list(rule["default"])
    return {"domains": domains, "matched": matched, "file_name": fn.group(0) if fn else None,
            "default_used": not matched}


def lint_request(request: str, catalog: Iterable, rule: dict, fixture_files: Iterable = (),
                 tool_domains: dict = None) -> dict:
    """Returns {"ok", "group", "codes", "messages", "detail", "derived"}. ``fixture_files`` are the
    file names (base names) in the task's fixture. ``tool_domains`` is the adapter TOOL_DOMAINS map,
    used only for DOMAIN_FILE_TOOLS (pass None to skip that clarification)."""
    text = request.lower()
    catalog = sorted(set(catalog))
    codes: list = []
    detail: dict = {}

    def hit(code: str, what) -> None:
        if code not in codes:
            codes.append(code)
        detail.setdefault(code, []).append(what)

    for name in catalog:
        if _word(text, name.lower()):
            hit("LEAK_TOOL_NAME", name)
    exempt = all_keywords(rule)
    for tok in sorted(name_tokens(catalog) - exempt):
        if _word(text, tok):
            hit("LEAK_TOOL_TOKEN", tok)
    for w in ALWAYS_BANNED:
        if _word(text, w) or _word(text, w + "s"):
            hit("BANNED_WORD", w)
    for ext in filename_extensions(rule):
        if re.search(re.escape(ext) + r"(?![a-z0-9_])", text):
            hit("EXTENSION", ext)
    if "/" in request or "\\" in request:
        hit("PATH_SEPARATOR", "/" if "/" in request else "\\")
    for fname in sorted(set(fixture_files)):
        if fname and _word(text, fname.lower()):
            hit("FIXTURE_NAME", fname)

    derived = derive_match(request, rule)
    group = None
    domains = derived["domains"]
    if derived["default_used"]:
        group = "default"
    elif "filesystem" in derived["matched"]:
        hit("DOMAIN_FILE_KEYWORD", derived["matched"]["filesystem"] or derived["file_name"])
    if group is None and len(domains) > 1:
        hit("DOMAIN_MULTI", domains)
    if group is None and "DOMAIN_FILE_KEYWORD" not in codes and len(domains) == 1:
        d = domains[0]
        if tool_domains is not None and any(d in tool_domains.get(t, ()) for t in WORKSPACE_READ_TOOLS):
            hit("DOMAIN_FILE_TOOLS", d)
        else:
            group = "nonfile"
    ok = not codes
    return {"ok": ok, "group": group if ok else None, "codes": codes,
            "messages": [AUTHOR_MESSAGES[c] for c in codes], "detail": detail, "derived": derived}


# ------------------------------------------------------------------------------- replay of the log

def replay(entries: list, catalog: Iterable, rule: dict, fixtures: dict, tool_domains: dict = None,
           groups: tuple = ("nonfile", "default"), target: dict = None, first_pass: dict = None,
           brief_for_group: dict = None) -> dict:
    """Re-derive every decision of an authoring-log set and return {"errors", "accepted"}.

    Each entry: {"id": "q-0001", "round": 0..3, "brief": str, "request": str, "fixture": name,
                 "lint": [author messages], "codes": [...], "group": ..., "status": ...}.
    Decisions, in id order (id order is authoring order, top-up ids continue the numbering):
      * lint again from the recorded text; ``codes``, ``group`` and ``lint`` must equal the recorded ones;
      * ``status`` is "accepted" for a lint-passing request whose group has fewer than
        ``target[group]`` accepted so far (default 24 each), "discarded_lint" for a rejected one,
        and "discarded_surplus" for a passing one beyond the group size (kept in the log, never run);
      * round r >= 1 (a top-up) may only use the brief of a group that was short after the rounds
        before it, and there are at most MAX_TOP_UPS such rounds;
      * round 0 has exactly ``first_pass[brief]`` requests per brief (default 36 each, two briefs).
    ``fixtures`` maps a fixture name to its file base names. With ``groups=("any",)`` (the calibration
    set, which has no group split) every passing request counts into one pool."""
    target = target or {g: GROUP_SIZE for g in groups}
    pooled = groups == ("any",)
    errors: list = []
    ids = [e["id"] for e in entries]
    if len(set(ids)) != len(ids):
        errors.append("authoring log has duplicate request ids")
    ordered = sorted(entries, key=lambda e: e["id"])
    if [e["id"] for e in entries] != [e["id"] for e in ordered]:
        errors.append("authoring log is not in request-id order")
    rounds = sorted({e.get("round", 0) for e in ordered})
    if rounds and rounds != list(range(len(rounds))):
        errors.append(f"authoring rounds {rounds} are not 0..n without gaps")
    if len(rounds) - 1 > MAX_TOP_UPS:
        errors.append(f"{len(rounds) - 1} top-up rounds, at most {MAX_TOP_UPS} allowed")
    prev = -1
    for e in ordered:
        if e.get("round", 0) < prev:
            errors.append(f"{e['id']}: round {e.get('round')} after round {prev} (ids must follow authoring order)")
        prev = max(prev, e.get("round", 0))

    briefs0: dict = {}
    for e in ordered:
        if e.get("round", 0) == 0:
            briefs0[e.get("brief")] = briefs0.get(e.get("brief"), 0) + 1
    want = first_pass if first_pass is not None else {"everyday": FIRST_PASS_PER_BRIEF, "question": FIRST_PASS_PER_BRIEF}
    if briefs0 != want:
        errors.append(f"first pass has {briefs0} requests per brief, expected {want}")

    count = {g: 0 for g in groups}
    short_before = {0: set(groups)}
    brief_for_group = brief_for_group or {"nonfile": "everyday", "default": "question"}
    current_round = 0
    accepted: list = []
    for e in ordered:
        r = e.get("round", 0)
        if r != current_round:
            current_round = r
            short_before[r] = {g for g in groups if count[g] < target[g]}
            if not short_before[r]:
                errors.append(f"{e['id']}: top-up round {r} although no group was short")
        allowed = set(briefs0) if pooled else {brief_for_group[g] for g in short_before[r] if g in brief_for_group}
        if r >= 1 and e.get("brief") not in allowed:
            errors.append(f"{e['id']}: top-up brief {e.get('brief')!r} does not match a short group")
        res = lint_request(e["request"], catalog, rule, fixtures.get(e.get("fixture"), ()), tool_domains)
        if e.get("codes") != res["codes"]:
            errors.append(f"{e['id']}: recorded lint codes {e.get('codes')} != re-derived {res['codes']}")
        if e.get("lint") != res["messages"]:
            errors.append(f"{e['id']}: recorded lint messages differ from the lint's")
        if e.get("group") != res["group"]:
            errors.append(f"{e['id']}: recorded group {e.get('group')!r} != re-derived {res['group']!r}")
        key = "any" if pooled else res["group"]
        if not res["ok"]:
            status = "discarded_lint"
        elif key in count and count[key] < target[key]:
            status = "accepted"
            count[key] += 1
            accepted.append(e["id"])
        else:
            status = "discarded_surplus"
        if e.get("status") != status:
            errors.append(f"{e['id']}: recorded status {e.get('status')!r} != re-derived {status!r}")
    for g in groups:
        if count[g] != target[g]:
            errors.append(f"group {g}: {count[g]} accepted, need exactly {target[g]}")
    return {"errors": errors, "accepted": accepted, "counts": count}
