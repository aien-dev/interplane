"""Simulated tool backends for the INTERPLANE bench, version sim-1 (stdlib only; no model, no GPU).

The registry is data: ``bench/stubs/backends.json`` names, for every capability a bench task lists,
which backend answers it once Odysseus has AUTHORIZED the call, and what that backend does. This
module implements the ``computed`` and ``declared_failure`` kinds. ``executable`` tools run in the
reference adapter and ``odysseus_handler`` tools run Odysseus's own handler; both are dispatched by
``run_bench.BenchAuthority``, which needs the Odysseus venv. Nothing here decides anything: the
decision is always Odysseus's, made before this code is reached.

Determinism: the same fixture, store file and argument sequence give the same raw results, byte
for byte. Private-data tools read a per-fixture store (``bench/stubs/stores/<fixture>.json``; a
fixture without one has empty stores) and write only to a run-local copy of it. Workspace write
tools change only the run's own copy of the fixture, confined to it. Ids made by a run are
``sim-<kind>-<n>`` in creation order.

Raw results have the shape Odysseus's handlers use: ``{"output": str, "exit_code": 0}`` on
success, ``{"error": str, "exit_code": 1}`` on failure.
"""

from __future__ import annotations

import copy
import json
import os
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
BACKENDS_PATH = BENCH / "stubs" / "backends.json"
STORES_DIR = BENCH / "stubs" / "stores"
VERSION = "sim-1"
KINDS = ("executable", "odysseus_handler", "computed", "declared_failure")
STORE_KEYS = ("notes", "memory", "calendar", "contacts", "documents", "models", "tokens")


def load_backends(path: Path = BACKENDS_PATH) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def store_path(fixture: str) -> Path:
    """``fixtures/lanternfish`` -> ``bench/stubs/stores/lanternfish.json``."""
    return STORES_DIR / (Path(fixture).name + ".json")


def load_store(fixture: str) -> dict:
    p = store_path(fixture)
    data = json.loads(p.read_text(encoding="utf-8")) if p.is_file() else {}
    return {k: copy.deepcopy(data.get(k, [])) for k in STORE_KEYS}


def ok(text: str) -> dict:
    return {"output": text, "exit_code": 0}


def fail(text: str) -> dict:
    return {"error": text, "exit_code": 1}


def _s(args: dict, key: str) -> str:
    v = args.get(key)
    return v.strip() if isinstance(v, str) else ""


def _has(hay: str, needle: str) -> bool:
    return needle.casefold() in hay.casefold()


# ------------------------------------------------------------------------------- workspace paths

def confine(workspace: str, raw: str):
    """(absolute path, None) inside the workspace, or (None, refusal). Symlinks are resolved."""
    if not raw:
        return None, "path is required"
    root = os.path.realpath(workspace)
    target = os.path.realpath(raw if os.path.isabs(raw) else os.path.join(root, raw))
    if target != root and not target.startswith(root + os.sep):
        return None, f"path {raw!r} is outside the workspace"
    return target, None


class SimSession:
    """One run's simulated backends: a fresh store copy and the run's workspace."""

    def __init__(self, workspace: str, fixture: str, backends: dict | None = None):
        self.workspace = workspace
        self.backends = backends if backends is not None else load_backends()
        self.store = load_store(fixture)
        self.outbox: list = []
        self.counters: dict = {}

    def kind(self, name: str):
        entry = self.backends["tools"].get(name)
        return entry["kind"] if entry else None

    def _new_id(self, what: str) -> str:
        self.counters[what] = self.counters.get(what, 0) + 1
        return f"sim-{what}-{self.counters[what]}"

    def call(self, name: str, args: dict) -> dict:
        """Raw result for an AUTHORIZED call to a ``computed`` or ``declared_failure`` tool."""
        entry = self.backends["tools"].get(name)
        if entry is None or entry["kind"] not in ("computed", "declared_failure"):
            raise KeyError(f"{name}: no computed or declared_failure backend")
        if entry["kind"] == "declared_failure":
            return fail(entry["message"])
        return SIMS[entry["sim"]](self, args if isinstance(args, dict) else {})


# ------------------------------------------------------------------------------- simulators

def sim_user_unavailable(s: SimSession, args: dict) -> dict:
    return ok("No user is available to answer in this benchmark run. Continue with the information "
              "you have and state any assumption in your final answer.")


def _note_line(n: dict) -> str:
    return f"- [{n['id']}] {n.get('title') or '(untitled)'}" + (f" (label: {n['label']})" if n.get("label") else "")


def _note_body(n: dict) -> str:
    if n.get("note_type") == "checklist":
        body = "\n".join(f"[{'x' if i.get('done') else ' '}] {i.get('text', '')}" for i in n.get("checklist_items", []))
    else:
        body = n.get("content", "")
    return f"{n.get('title') or '(untitled)'} [{n['id']}]\n{body}"


def _by_prefix(items: list, key: str, ident: str):
    hits = [x for x in items if ident and str(x.get(key, "")).startswith(ident)]
    return hits[0] if len(hits) == 1 else None


def sim_notes(s: SimSession, args: dict) -> dict:
    notes = s.store["notes"]
    action = _s(args, "action")
    if action in ("list", "search"):
        q = _s(args, "query")
        if action == "search" and not q:
            return fail("manage_notes: search needs a query")
        show_archived = args.get("archived") is True
        label = _s(args, "label")
        hits = [n for n in notes if bool(n.get("archived")) == show_archived
                and (not label or n.get("label") == label)
                and (not q or _has(n.get("title", "") + "\n" + _note_body(n), q))]
        if not hits:
            return ok("No notes match." if q else "No notes.")
        return ok(f"{len(hits)} note(s):\n" + "\n".join(_note_line(n) for n in hits))
    if action == "view":
        n = _by_prefix(notes, "id", _s(args, "id"))
        return ok(_note_body(n)) if n else fail(f"manage_notes: no note with id {_s(args, 'id')!r}")
    if action == "add":
        n = {"id": s._new_id("note"), "title": _s(args, "title"), "note_type": _s(args, "note_type") or "note",
             "content": args.get("content") if isinstance(args.get("content"), str) else "",
             "checklist_items": args.get("checklist_items") if isinstance(args.get("checklist_items"), list) else [],
             "label": _s(args, "label"), "archived": False}
        notes.append(n)
        return ok(f"Created note {n['id']}.")
    if action in ("update", "delete", "toggle_item"):
        n = _by_prefix(notes, "id", _s(args, "id"))
        if n is None:
            return fail(f"manage_notes: no note with id {_s(args, 'id')!r}")
        if action == "delete":
            notes.remove(n)
            return ok(f"Deleted note {n['id']}.")
        if action == "toggle_item":
            i = args.get("index")
            items = n.get("checklist_items", [])
            if not isinstance(i, int) or not 0 <= i < len(items):
                return fail("manage_notes: index out of range")
            items[i]["done"] = not items[i].get("done")
            return ok(f"Toggled item {i} of note {n['id']}.")
        for key in ("title", "content", "label"):
            if isinstance(args.get(key), str):
                n[key] = args[key]
        if isinstance(args.get("archived"), bool):
            n["archived"] = args["archived"]
        return ok(f"Updated note {n['id']}.")
    return fail(f"manage_notes: unknown action {action!r}")


def sim_memory(s: SimSession, args: dict) -> dict:
    mem = s.store["memory"]
    action = _s(args, "action")
    if action in ("list", "search"):
        cat, q = _s(args, "category"), _s(args, "text")
        if action == "search" and not q:
            return fail("manage_memory: search needs text")
        hits = [m for m in mem if (not cat or m.get("category") == cat) and (action != "search" or _has(m.get("text", ""), q))]
        if not hits:
            return ok("No memories match." if action == "search" else "No memories.")
        return ok(f"{len(hits)} memory(ies):\n" + "\n".join(f"- [{m['id']}] ({m.get('category', 'fact')}) {m.get('text', '')}" for m in hits))
    if action == "add":
        text = _s(args, "text")
        if not text:
            return fail("manage_memory: add needs text")
        m = {"id": s._new_id("memory"), "category": _s(args, "category") or "fact", "text": text}
        mem.append(m)
        return ok(f"Saved memory {m['id']}.")
    if action in ("edit", "delete"):
        m = _by_prefix(mem, "id", _s(args, "memory_id"))
        if m is None:
            return fail(f"manage_memory: no memory with id {_s(args, 'memory_id')!r}")
        if action == "delete":
            mem.remove(m)
            return ok(f"Deleted memory {m['id']}.")
        if _s(args, "text"):
            m["text"] = _s(args, "text")
        return ok(f"Updated memory {m['id']}.")
    return fail(f"manage_memory: unknown action {action!r}")


def _event_line(e: dict) -> str:
    return f"- [{e['uid']}] {e.get('dtstart', '')} {e.get('summary', '')}" + (f" @ {e['location']}" if e.get("location") else "")


def sim_calendar(s: SimSession, args: dict) -> dict:
    events = s.store["calendar"]
    action = _s(args, "action")
    if action == "list_calendars":
        return ok("1 calendar:\n- Personal (default)")
    if action == "list_events":
        start, end = _s(args, "start"), _s(args, "end")
        hits = [e for e in events
                if (not start or e.get("dtstart", "")[:10] >= start[:10])
                and (not end or e.get("dtstart", "")[:10] <= end[:10])]
        hits.sort(key=lambda e: (e.get("dtstart", ""), e["uid"]))
        return ok(f"{len(hits)} event(s):\n" + "\n".join(_event_line(e) for e in hits)) if hits else ok("No events in that range.")
    if action == "create_event":
        if not _s(args, "summary") or not _s(args, "dtstart"):
            return fail("manage_calendar: create_event needs summary and dtstart")
        e = {"uid": s._new_id("event"), "summary": _s(args, "summary"), "dtstart": _s(args, "dtstart"), "location": _s(args, "location")}
        events.append(e)
        return ok(f"Created event {e['uid']}.")
    if action in ("update_event", "delete_event"):
        e = _by_prefix(events, "uid", _s(args, "uid"))
        if e is None:
            return fail(f"manage_calendar: no event with uid {_s(args, 'uid')!r}")
        if action == "delete_event":
            events.remove(e)
            return ok(f"Deleted event {e['uid']}.")
        for key in ("summary", "dtstart", "location"):
            if _s(args, key):
                e[key] = _s(args, key)
        return ok(f"Updated event {e['uid']}.")
    return fail(f"manage_calendar: unknown action {action!r}")


def _contact_line(c: dict) -> str:
    parts = [", ".join(c.get("emails", []))] + [", ".join(c.get("phones", []))]
    return f"- [{c['uid']}] {c['name']}: " + "; ".join(p for p in parts if p)


def sim_contacts(s: SimSession, args: dict) -> dict:
    cs = s.store["contacts"]
    action = _s(args, "action")
    if action == "list":
        return ok(f"{len(cs)} contact(s):\n" + "\n".join(_contact_line(c) for c in cs)) if cs else ok("No contacts.")
    if action == "add":
        if not _s(args, "name"):
            return fail("manage_contact: add needs a name")
        emails = [x for x in args.get("emails", []) if isinstance(x, str)] if isinstance(args.get("emails"), list) else []
        if _s(args, "email") and _s(args, "email") not in emails:
            emails.insert(0, _s(args, "email"))
        phones = [x for x in args.get("phones", []) if isinstance(x, str)] if isinstance(args.get("phones"), list) else []
        if not (emails or phones or _s(args, "address")):
            return fail("manage_contact: add needs an email, phone or address")
        c = {"uid": s._new_id("contact"), "name": _s(args, "name"), "emails": emails, "phones": phones}
        cs.append(c)
        return ok(f"Added contact {c['uid']}.")
    if action in ("update", "delete"):
        c = _by_prefix(cs, "uid", _s(args, "uid"))
        if c is None:
            return fail(f"manage_contact: no contact with uid {_s(args, 'uid')!r}; call action='list' first")
        if action == "delete":
            cs.remove(c)
            return ok(f"Deleted contact {c['uid']}.")
        if _s(args, "name"):
            c["name"] = _s(args, "name")
        if _s(args, "email"):
            c["emails"] = [_s(args, "email")] + [x for x in c.get("emails", []) if x != _s(args, "email")]
        return ok(f"Updated contact {c['uid']}.")
    return fail(f"manage_contact: unknown action {action!r}")


def sim_resolve_contact(s: SimSession, args: dict) -> dict:
    name = _s(args, "name")
    if not name:
        return fail("resolve_contact: name is required")
    hits = [c for c in s.store["contacts"] if _has(c["name"], name)]
    if not hits:
        return ok(f"No contact found for {name!r}.")
    return ok("\n".join(f"{c['name']}: {', '.join(c.get('emails') or c.get('phones') or ['no address'])} (source: CardDAV)" for c in hits))


def sim_documents(s: SimSession, args: dict) -> dict:
    docs = s.store["documents"]
    action = _s(args, "action")
    if action == "list":
        q, lang = _s(args, "search"), _s(args, "language")
        lim = args.get("limit") if isinstance(args.get("limit"), int) and args.get("limit") > 0 else 50
        hits = [d for d in docs if (not q or _has(d.get("title", "") + "\n" + d.get("snippet", ""), q))
                and (not lang or d.get("language") == lang)][:lim]
        if not hits:
            return ok("No editor documents match." if q else "No editor documents.")
        return ok(f"{len(hits)} document(s):\n" + "\n".join(f"- [{d['id']}] {d.get('title', '')} ({d.get('language', 'text')})" for d in hits))
    if action == "delete":
        d = _by_prefix(docs, "id", _s(args, "document_id"))
        if d is None:
            return fail(f"manage_documents: no document with id {_s(args, 'document_id')!r}")
        docs.remove(d)
        return ok(f"Deleted document {d['id']}.")
    if action == "tidy":
        return ok("Nothing to tidy.")
    return fail(f"manage_documents: unknown action {action!r}")


def sim_models(s: SimSession, args: dict) -> dict:
    f = _s(args, "filter")
    hits = [m for m in s.store["models"] if not f or _has(m.get("name", "") + " " + m.get("endpoint", ""), f)]
    if not hits:
        return ok("No models match." if f else "No models configured.")
    return ok(f"{len(hits)} model(s):\n" + "\n".join(f"- {m['name']} (endpoint: {m.get('endpoint', 'local')})" for m in hits))


def sim_tokens(s: SimSession, args: dict) -> dict:
    toks = s.store["tokens"]
    action = _s(args, "action")
    if action == "list":
        return ok(f"{len(toks)} token(s):\n" + "\n".join(f"- [{t['id']}] {t['name']}" for t in toks)) if toks else ok("No API tokens.")
    if action == "create":
        if not _s(args, "name"):
            return fail("manage_tokens: create needs a name")
        t = {"id": s._new_id("token"), "name": _s(args, "name")}
        toks.append(t)
        return ok(f"Created token {t['id']} named {t['name']!r}. The secret is not shown in the benchmark.")
    if action == "delete":
        t = _by_prefix(toks, "id", _s(args, "token_id"))
        if t is None:
            return fail(f"manage_tokens: no token with id {_s(args, 'token_id')!r}")
        toks.remove(t)
        return ok(f"Deleted token {t['id']}.")
    return fail(f"manage_tokens: unknown action {action!r}")


def sim_bg_jobs(s: SimSession, args: dict) -> dict:
    action = _s(args, "action") or "list"
    if action == "list":
        return ok("No background jobs in this chat.")
    if action in ("output", "kill"):
        return fail(f"manage_bg_jobs: no job with id {_s(args, 'job_id')!r}")
    return fail(f"manage_bg_jobs: unknown action {action!r}")


def sim_send_email(s: SimSession, args: dict) -> dict:
    to, subject = _s(args, "to"), _s(args, "subject")
    if not to or "@" not in to:
        return fail("send_email: 'to' must be an email address")
    s.outbox.append({"to": to, "subject": subject})
    return ok(f"Email to {to} queued as message {s._new_id('mail')} (simulated: recorded, not delivered).")


def sim_write_file(s: SimSession, args: dict) -> dict:
    path, why = confine(s.workspace, _s(args, "path"))
    if path is None:
        return fail(f"write_file: {why}")
    content = args.get("content")
    if not isinstance(content, str):
        return fail("write_file: content must be a string")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)
    return ok(f"Wrote {len(content.encode('utf-8'))} bytes to {os.path.relpath(path, os.path.realpath(s.workspace))}.")


def sim_edit_file(s: SimSession, args: dict) -> dict:
    path, why = confine(s.workspace, _s(args, "path"))
    if path is None:
        return fail(f"edit_file: {why}")
    old, new = args.get("old_string"), args.get("new_string")
    if not isinstance(old, str) or not isinstance(new, str) or not old:
        return fail("edit_file: old_string and new_string must be strings and old_string non-empty")
    if not os.path.isfile(path):
        return fail(f"edit_file: file not found: {_s(args, 'path')}")
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    n = text.count(old)
    if n == 0:
        return fail("edit_file: old_string not found in the file")
    if n > 1 and args.get("replace_all") is not True:
        return fail(f"edit_file: old_string occurs {n} times; pass replace_all or a longer old_string")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.replace(old, new) if args.get("replace_all") is True else text.replace(old, new, 1))
    return ok(f"Edited {os.path.relpath(path, os.path.realpath(s.workspace))}: {n if args.get('replace_all') is True else 1} replacement(s).")


def sim_apply_patch(s: SimSession, args: dict) -> dict:
    """Subset of the *** Begin Patch format: Add File, Delete File, Update File with -/+/space lines.

    All sections are checked before anything is written, so a failing patch changes nothing.
    """
    text = args.get("patch_text")
    if not isinstance(text, str):
        return fail("apply_patch: patch_text must be a string")
    lines = text.strip("\n").split("\n")
    if len(lines) < 2 or lines[0].strip() != "*** Begin Patch" or lines[-1].strip() != "*** End Patch":
        return fail("apply_patch: patch must start with *** Begin Patch and end with *** End Patch")
    sections, cur = [], None
    for ln in lines[1:-1]:
        for head, op in (("*** Add File: ", "add"), ("*** Delete File: ", "delete"), ("*** Update File: ", "update")):
            if ln.startswith(head):
                cur = {"op": op, "path": ln[len(head):].strip(), "body": []}
                sections.append(cur)
                break
        else:
            if cur is None:
                return fail("apply_patch: content before the first file section")
            cur["body"].append(ln)
    if not sections:
        return fail("apply_patch: no file sections")
    plan = []
    for sec in sections:
        path, why = confine(s.workspace, sec["path"])
        if path is None:
            return fail(f"apply_patch: {why}")
        if sec["op"] == "add":
            if os.path.exists(path):
                return fail(f"apply_patch: {sec['path']} already exists")
            if any(not b.startswith("+") for b in sec["body"]):
                return fail(f"apply_patch: Add File lines must start with '+' ({sec['path']})")
            plan.append((path, "\n".join(b[1:] for b in sec["body"]) + "\n"))
        elif sec["op"] == "delete":
            if not os.path.isfile(path):
                return fail(f"apply_patch: {sec['path']} not found")
            plan.append((path, None))
        else:
            if not os.path.isfile(path):
                return fail(f"apply_patch: {sec['path']} not found")
            with open(path, encoding="utf-8") as fh:
                cur_text = fh.read()
            old, new = [], []
            for b in sec["body"]:
                if b.startswith("@@"):
                    continue
                tag, rest = (b[:1], b[1:]) if b else (" ", "")
                if tag not in " -+":
                    return fail(f"apply_patch: bad hunk line in {sec['path']}: {b[:40]!r}")
                if tag in " -":
                    old.append(rest)
                if tag in " +":
                    new.append(rest)
            old_s, new_s = "\n".join(old), "\n".join(new)
            if not old_s or cur_text.count(old_s) != 1:
                return fail(f"apply_patch: hunk for {sec['path']} does not match the file exactly once")
            plan.append((path, cur_text.replace(old_s, new_s, 1)))
    for path, content in plan:
        if content is None:
            os.remove(path)
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(content)
    return ok(f"Applied patch: {len(plan)} file(s) changed.")


SIMS = {
    "user_unavailable": sim_user_unavailable,
    "notes": sim_notes,
    "memory": sim_memory,
    "calendar": sim_calendar,
    "contacts": sim_contacts,
    "resolve_contact": sim_resolve_contact,
    "documents": sim_documents,
    "models": sim_models,
    "tokens": sim_tokens,
    "bg_jobs": sim_bg_jobs,
    "send_email": sim_send_email,
    "write_file": sim_write_file,
    "edit_file": sim_edit_file,
    "apply_patch": sim_apply_patch,
}

# Sample calls for the offline working check (validate.py): every computed simulator is called
# twice on a fresh session per fixture and must give the same result both times, without raising.
SAMPLES = {
    "user_unavailable": [{"question": "Which file?", "options": [{"label": "a"}, {"label": "b"}]}],
    "notes": [{"action": "list"}, {"action": "search", "query": "release"}, {"action": "add", "title": "t", "content": "c"},
              {"action": "view", "id": "sim-note-1"}],
    "memory": [{"action": "list"}, {"action": "search", "text": "station"}, {"action": "add", "text": "x"}],
    "calendar": [{"action": "list_calendars"}, {"action": "list_events", "start": "2026-01-01", "end": "2026-12-31"},
                 {"action": "create_event", "summary": "s", "dtstart": "2026-11-01"}],
    "contacts": [{"action": "list"}, {"action": "add", "name": "N", "email": "n@example.org"}],
    "resolve_contact": [{"name": "Reyes"}, {"name": "Nobody"}],
    "documents": [{"action": "list"}, {"action": "list", "search": "tide"}, {"action": "tidy"}],
    "models": [{}, {"filter": "tide"}],
    "tokens": [{"action": "list"}, {"action": "create", "name": "ci-bot"}],
    "bg_jobs": [{"action": "list"}, {"action": "output", "job_id": "1"}],
    "send_email": [{"to": "a@example.org", "subject": "s", "body": "b"}],
    "write_file": [{"path": "SIM-CHECK.txt", "content": "x\n"}, {"path": "../outside.txt", "content": "x"}],
    "edit_file": [{"path": "SIM-CHECK.txt", "old_string": "x", "new_string": "y"}],
    "apply_patch": [{"patch_text": "*** Begin Patch\n*** Add File: SIM-PATCH.txt\n+hello\n*** End Patch"}],
}
