"""Offline tests of the 0.2y leakage lint, authoring-log replay and corpus checks (stdlib only; no model, no GPU).

  python3 bench/tools/test_lint_02y.py

Each rejection rule of PREREG-0.2y section 5 is shown firing on a counterexample, a clean request passes,
the acceptance replay (24 / 24, id order, at most three top-ups) is exercised on synthetic logs, and a
fully synthetic 120-task corpus goes through the same checks ``validate.py --corpus 0.2y`` runs.
"""

from __future__ import annotations

import copy
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
sys.path.insert(0, str(HERE))
import lint_02y as L  # noqa: E402
import validate as V
import assemble_02y as A  # noqa: E402

RULE = V.load_rule()
CATALOG = sorted(c["name"] for c in json.loads(V.CATALOG_PATH.read_text(encoding="utf-8"))["capabilities"])
TDOM = V.tool_domains()


def lint(request, fixture_files=()):
    return L.lint_request(request, CATALOG, RULE, fixture_files, TDOM)


class RejectionRules(unittest.TestCase):
    def codes(self, request, fixture_files=()):
        return lint(request, fixture_files)["codes"]

    def test_clean_requests_pass_in_their_group(self):
        r = lint("Check my email about the lighthouse")
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["group"], "nonfile")
        self.assertEqual(r["derived"]["domains"], ["email"])
        r = lint("Which supplier does the harbour crew order lanterns from?")
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["group"], "default")
        self.assertEqual(r["derived"]["domains"], ["filesystem"])
        self.assertTrue(r["derived"]["default_used"])

    def test_catalog_name_rejected(self):
        self.assertIn("LEAK_TOOL_NAME", self.codes("Use read_file now"))
        self.assertIn("LEAK_TOOL_NAME", self.codes("Can you glob that for me?"))
        self.assertIn("LEAK_TOOL_NAME", self.codes("Run bash on it"))

    def test_name_token_rejected_unless_keyword(self):
        self.assertEqual(self.codes("Please manage the board"), ["LEAK_TOOL_TOKEN"])
        self.assertEqual(lint("Please manage the board")["detail"]["LEAK_TOOL_TOKEN"], ["manage"])
        self.assertIn("LEAK_TOOL_TOKEN", self.codes("Which supplier is it with?"))  # "with" is a token of chat_with_model
        # keyword exemption: email, calendar, notes, models, tasks, chat are name tokens AND keywords
        for ok in ("Check my email", "What is on my calendar", "Any notes on the harbour", "Which models are local",
                   "Which tasks are open", "Who started the chat"):
            self.assertTrue(lint(ok)["ok"], ok)

    def test_banned_words_stay_banned_even_as_keywords(self):
        for req in ("Open the file for me", "Which files changed", "What is the path", "Use grep", "Use the search tool",
                    "Which tool does it", "Which function is it", "What command did I use", "Which commands exist"):
            self.assertIn("BANNED_WORD", self.codes(req), req)
        self.assertIn("file", RULE["keywords"]["filesystem"])  # file is a keyword, still banned

    def test_extension_rejected(self):
        for req in ("What is in the rota.md", "See config.toml", "Open the .rst", "Look in plan.tsv", "Check page.html", "Edit data.xml"):
            self.assertIn("EXTENSION", self.codes(req), req)
        self.assertNotIn("EXTENSION", self.codes("Who wrote it. Then who read it"))

    def test_path_separator_rejected(self):
        self.assertIn("PATH_SEPARATOR", self.codes("Who owns docs/rota"))
        self.assertIn("PATH_SEPARATOR", self.codes("Who owns docs\\rota"))
        self.assertIn("PATH_SEPARATOR", self.codes("Dock and/or berth"))

    def test_fixture_file_name_rejected(self):
        self.assertEqual(self.codes("What does the LICENSE say about reuse?", ["LICENSE"]), ["FIXTURE_NAME"])
        self.assertNotIn("FIXTURE_NAME", self.codes("What does the LICENSE say about reuse?", ["rota"]))
        self.assertIn("FIXTURE_NAME", self.codes("Who is in keeper-rota", ["keeper-rota"]))

    def test_domain_rules(self):
        # filesystem keyword (not the default list) is rejected
        r = lint("Who owns the project?")
        self.assertEqual(r["codes"], ["DOMAIN_FILE_KEYWORD"])
        self.assertIsNone(r["group"])
        # more than one domain is rejected
        self.assertEqual(self.codes("Is there a meeting or an email about it?"), ["DOMAIN_MULTI"])
        # clarification: code is a non-file domain whose tools include the workspace-read tools
        self.assertEqual(self.codes("Where is the source of the crash?"), ["DOMAIN_FILE_TOOLS"])
        self.assertTrue(L.lint_request("Where is the source of the crash?", CATALOG, RULE, (), None)["ok"])  # lint without the clarification
        # a file-name-shaped token derives the filesystem domain
        r = lint("Who wrote rota.ini")
        self.assertIn("DOMAIN_FILE_KEYWORD", r["codes"])

    def test_adversarial_spellings_are_rejected(self):
        base = "How many lanterns arrive on Friday"
        self.assertEqual(lint(base + "?")["codes"], [])
        # each variant has no other trigger than the spelling; the exact code is asserted
        cases = [
            ("How many F I L E lanterns arrive on Friday?", "BANNED_WORD"),
            ("How many lanterns arrive in the f.i.l.e on Friday?", "BANNED_WORD"),
            ("How many my_file_q3 lanterns arrive on Friday?", "BANNED_WORD"),
            ("How many lanterns arrive on Friday? P A T H", "BANNED_WORD"),
            ("How many lanterns arrive on Friday? t-o-o-l", "BANNED_WORD"),
            ("How many f\u0456le lanterns arrive on Friday?", "NON_ASCII"),
            ("How many fi\u200ble lanterns arrive on Friday?", "NON_ASCII"),
            ("How many lanterns f\u0301ile arrive on Friday?", "NON_ASCII"),
            ("How many lanterns f\u0903ile arrive on Friday?", "NON_ASCII"),
            (base + " a\uff0fb?", "PATH_SEPARATOR"),
            (base + " a\u2215b?", "PATH_SEPARATOR"),
            (base + " a\u29f8b?", "PATH_SEPARATOR"),
            (base + " a\u2216b?", "PATH_SEPARATOR"),
            (base + "? R E A D _ F I L E", "LEAK_TOOL_NAME"),
        ]
        for req, code in cases:
            r = lint(req)
            self.assertFalse(r["ok"], (req, r["codes"]))
            self.assertEqual(r["codes"], [code], req)
        self.assertIn("NON_ASCII", self.codes("Check my \uff46ile"))
        self.assertTrue(lint("Check my email about the lighthouse")["ok"])
        self.assertTrue(lint("Which supplier does the harbour crew order lanterns from?")["ok"])
        self.assertTrue(lint("Who wrote the company profile?")["ok"])  # no squashed-substring check: "profile" is fine

    def test_naming_the_everyday_area_reaches_the_non_file_group(self):
        # amendment 4 (brief E may name the everyday thing): attempt 2 facts asked bare land in the default
        # group; the same facts with the area named pass into the non-file group
        pairs = [
            ("Who approved the blue enamel sample?", "In my mail, who approved the blue enamel sample?"),
            ("Where did polishing move after the huddle?", "What did the meeting decide about polishing?"),
            ("Who is our quay courier?", "Who is our quay courier in my contacts?"),
            ("What's the status of the plum shade?", "What did my notes say about the plum shade?"),
            ("What day is the owl costume fitting confirmed for?",
             "In the chat, what day did we confirm for the owl fitting?"),
        ]
        for bare, named in pairs:
            self.assertEqual((lint(bare)["ok"], lint(bare)["group"]), (True, "default"), bare)
            self.assertEqual((lint(named)["ok"], lint(named)["group"]), (True, "nonfile"), named)

    def test_known_limits_are_accepted(self):
        # documented in the section 11 amendment: single-space splits inside a word and run-together forms
        self.assertEqual(lint("How many fil e lanterns arrive on Friday?")["codes"], [])
        self.assertEqual(lint("How many lanterns arrive on Friday? filepath")["codes"], [])

    def test_selector_codes_share_one_generic_message(self):
        msgs = {L.AUTHOR_MESSAGES[c] for c in ("DOMAIN_FILE_KEYWORD", "DOMAIN_MULTI", "DOMAIN_FILE_TOOLS")}
        self.assertEqual(msgs, {"Set aside by an automatic check."})
        self.assertEqual(lint("Who owns the project?")["messages"], ["Set aside by an automatic check."])
        self.assertEqual(lint("Who owns the project?")["codes"], ["DOMAIN_FILE_KEYWORD"])  # the precise code stays in the log

    def test_messages_never_reveal_words(self):
        r = lint("Please manage the board, see notes.md")
        self.assertTrue(r["messages"])
        for m in r["messages"]:
            for w in ("manage", "notes", ".md", "board"):
                self.assertNotIn(w, m.lower())
        self.assertIn("manage", json.dumps(r["detail"]))  # the detail (log only) carries the token

    def test_extensions_come_from_the_rule(self):
        exts = L.filename_extensions(RULE)
        for e in (".md", ".json", ".env", ".rst", ".adoc", ".org", ".tsv", ".conf", ".xml", ".html"):
            self.assertIn(e, exts)


# ------------------------------------------------------------------------------- replay

NONFILE_AREAS = ["email", "meeting", "contact", "calendar", "note", "todo", "chat", "model", "remember", "image", "document", "inbox"]


def nonfile_request(i):
    return f"Check my {NONFILE_AREAS[i % len(NONFILE_AREAS)]} about the lantern crew {chr(97 + i % 26)}{i}"


def default_request(i):
    return f"Which supplier does crew {chr(97 + i % 26)}{i} order lanterns from?"


def make_entries(nonfile=36, default=36, bad=0, start=1, rnd=0, brief_n="everyday", brief_d="question"):
    """Interleaved synthetic requests, ids q-0001..., optionally with ``bad`` discarded requests first."""
    raw = []
    for i in range(bad):
        raw.append((brief_n, f"Use read_file to check item {i}"))
    for i in range(nonfile):
        raw.append((brief_n, nonfile_request(i)))
    for i in range(default):
        raw.append((brief_d, default_request(i)))
    out = []
    for k, (brief, text) in enumerate(raw):
        out.append({"id": f"q-{start + k:04d}", "round": rnd, "brief": brief, "request": text, "fixture": "f1", "set": "target"})
    return out


def decide(entries, target=None, groups=("nonfile", "default")):
    """Fill codes, messages, group, status the way the authoring run does (the log writer)."""
    target = target or {g: 24 for g in groups}
    count = {g: 0 for g in groups}
    for e in entries:
        r = lint(e["request"], ["notes-fixture-file"])
        e["codes"], e["lint"], e["group"] = r["codes"], r["messages"], r["group"]
        key = "any" if groups == ("any",) else r["group"]
        if not r["ok"]:
            e["status"] = "discarded_lint"
        elif count.get(key, 99) < target.get(key, 0):
            e["status"] = "accepted"
            count[key] += 1
        else:
            e["status"] = "discarded_surplus"
    return entries


def replay(entries, **kw):
    return L.replay(entries, CATALOG, RULE, {"f1": ["notes-fixture-file"]}, TDOM, **kw)


def sized(n_nonfile, n_default):
    """A first pass of 36 everyday + 36 question requests whose lint outcomes give n_nonfile / n_default passing."""
    ents = []
    for i in range(36):
        text = nonfile_request(i) if i < n_nonfile else f"Use read_file to check item {i}"
        ents.append(("everyday", text))
    for i in range(36):
        text = default_request(i) if i < n_default else f"Use read_file to check item {i}"
        ents.append(("question", text))
    return [{"id": f"q-{k + 1:04d}", "round": 0, "brief": b, "request": t, "fixture": "f1", "set": "target"} for k, (b, t) in enumerate(ents)]


class Replay(unittest.TestCase):
    def test_happy_path_first_24_in_id_order_and_surplus_discarded(self):
        ents = decide(make_entries())
        res = replay(ents)
        self.assertEqual(res["errors"], [])
        self.assertEqual(res["counts"], {"nonfile": 24, "default": 24})
        self.assertEqual(res["accepted"], [e["id"] for e in ents if e["status"] == "accepted"])
        nonfile_ids = [e["id"] for e in ents if e["group"] == "nonfile"]
        self.assertEqual([e["id"] for e in ents if e["group"] == "nonfile" and e["status"] == "accepted"], nonfile_ids[:24])
        self.assertEqual(sum(e["status"] == "discarded_surplus" for e in ents), 24)

    def test_wrong_status_is_caught(self):
        ents = decide(make_entries())
        # pick a surplus request over an earlier accepted one: the id-order rule is violated
        first = next(e for e in ents if e["status"] == "accepted" and e["group"] == "nonfile")
        late = next(e for e in reversed(ents) if e["status"] == "discarded_surplus" and e["group"] == "nonfile")
        first["status"], late["status"] = "discarded_surplus", "accepted"
        errs = replay(ents)["errors"]
        self.assertTrue(any(first["id"] in x and "status" in x for x in errs), errs)
        self.assertTrue(any(late["id"] in x and "status" in x for x in errs), errs)

    def test_tampered_request_or_lint_is_caught(self):
        ents = decide(make_entries())
        ents[0]["request"] = "Use read_file to check it"
        errs = replay(ents)["errors"]
        self.assertTrue(any("lint codes" in x for x in errs), errs)
        ents = decide(make_entries())
        ents[1]["lint"] = []
        ents[1]["codes"] = []
        ents[1]["request"] = "Use read_file to check it"  # logged as clean but is not
        self.assertTrue(any("lint codes" in x for x in replay(ents)["errors"]))

    def test_short_group_fails_without_top_up(self):
        ents = decide(sized(36, 20))
        errs = replay(ents)["errors"]
        self.assertTrue(any("group default: 20 accepted, need exactly 24" in x for x in errs), errs)

    def top_up(self, nid, rnd, n, valid, brief="question"):
        out = []
        for i in range(n):
            text = default_request(200 + nid + i) if valid else f"Use read_file to check item {nid + i}"
            out.append({"id": f"q-{nid + i:04d}", "round": rnd, "brief": brief, "request": text, "fixture": "f1", "set": "target"})
        return out

    def test_one_top_up_fills_a_short_group_and_uses_the_matching_brief(self):
        ents = decide(sized(36, 20) + self.top_up(73, 1, 12, True))
        res = replay(ents)
        self.assertEqual(res["errors"], [])
        self.assertEqual(res["counts"]["default"], 24)

    def test_top_up_with_the_wrong_brief_or_no_shortage_fails(self):
        first = sized(36, 20)
        wrong = self.top_up(73, 1, 12, False, brief="everyday")
        self.assertTrue(any("does not match a short group" in x for x in replay(decide(first + wrong))["errors"]))
        full = make_entries()
        self.assertTrue(any("no group was short" in x for x in replay(decide(full + self.top_up(73, 1, 12, True)))["errors"]))

    def test_top_up_under_delivery_is_a_note(self):
        res = replay(decide(sized(36, 20) + self.top_up(73, 1, 6, True)))
        self.assertEqual(res["errors"], [])
        self.assertIn("top-up round 1, brief 'question': 6 requests returned, 12 asked", res["notes"])

    def test_an_empty_call_keeps_its_round_visible(self):
        ents = decide(sized(36, 0) + self.top_up(73, 1, 12, True))
        ents.append({"id": "q-0085", "round": 2, "brief": "question", "request": "", "fixture": "f1",
                     "set": "target", "status": "empty_call"})
        ents += decide(self.top_up(86, 3, 36, True))
        for e in ents[-36:]:
            e["status"] = "accepted" if e["id"] <= "q-0097" else "discarded_surplus"
        res = replay(ents)
        self.assertEqual(res["errors"], [])
        self.assertIn("top-up round 2, brief 'question': 0 requests returned, 24 asked", res["notes"])
        next(e for e in ents if e["id"] == "q-0085")["request"] = "Where is the depot rota kept?"  # a marker may not carry a request
        self.assertTrue(any("empty_call line must be the only line" in x for x in replay(ents)["errors"]))

    def test_over_delivery_beyond_the_asked_number_never_counts(self):
        # round 1 asks for 12: the first 12 are rejected by the lint, 6 valid ones follow beyond the ask
        ents = decide(sized(36, 20) + self.top_up(73, 1, 12, False) + self.top_up(85, 1, 6, True))
        errs = replay(ents)["errors"]
        self.assertTrue(any(x.startswith("q-0085: beyond the 12 asked in top-up round 1") for x in errs), errs)
        for e in ents[-6:]:
            e["status"] = "discarded_over_delivery"
        errs = replay(ents)["errors"]
        # marked correctly they are not linted into a group, so the default group stays short
        self.assertEqual(errs, ["group default: 20 accepted, need exactly 24"], errs)
        ents = decide(sized(36, 20) + self.top_up(73, 1, 12, True) + self.top_up(85, 1, 6, True))
        for e in ents[-6:]:
            e["status"] = "discarded_over_delivery"
        self.assertEqual(replay(ents)["errors"], [])
        ents[-7]["status"] = "discarded_over_delivery"  # one inside the ask may not be marked
        self.assertTrue(any(x.startswith("q-0084: recorded status") for x in replay(ents)["errors"]))

    def test_more_than_three_top_ups_fails(self):
        ents = sized(36, 0)
        nid = 73
        for rnd in range(1, 5):
            ents += self.top_up(nid, rnd, 12 * rnd, False)
            nid += 12 * rnd
        errs = replay(decide(ents))["errors"]
        self.assertTrue(any("4 top-up rounds, at most 3 allowed" in x for x in errs), errs)

    def test_three_top_ups_are_allowed(self):
        ents = sized(36, 5) + self.top_up(73, 1, 12, False) + self.top_up(85, 2, 24, False) + self.top_up(109, 3, 36, True)
        res = replay(decide(ents))
        self.assertEqual(res["errors"], [])
        self.assertEqual(res["counts"]["default"], 24)

    def test_unknown_fixture_is_an_error(self):
        ents = decide(make_entries())
        ents[0]["fixture"] = "nope"
        self.assertTrue(any("unknown fixture 'nope'" in x for x in replay(ents)["errors"]))

    def test_out_of_order_ids_and_bad_first_pass_fail(self):
        ents = decide(make_entries())
        ents[0], ents[1] = ents[1], ents[0]
        self.assertTrue(any("not in request-id order" in x for x in replay(ents)["errors"]))
        # amendment 3: a short first pass is a note, a missing brief is an error
        res = replay(decide(make_entries(nonfile=30, default=36)))
        self.assertEqual(res["errors"], [])
        self.assertIn("first pass, brief 'everyday': 30 requests returned, 36 asked", res["notes"])
        ents = [e for e in decide(make_entries()) if e["brief"] == "question"]
        self.assertTrue(any("first pass has briefs" in x for x in replay(ents)["errors"]))

    def test_calibration_pool_takes_the_first_30(self):
        ents = decide(make_entries(), target={"any": 30}, groups=("any",))
        res = L.replay(ents, CATALOG, RULE, {"f1": ["notes-fixture-file"]}, TDOM, groups=("any",), target={"any": 30})
        self.assertEqual(res["errors"], [])
        self.assertEqual(len(res["accepted"]), 30)


# ------------------------------------------------------------------------------- composition arithmetic

class Composition(unittest.TestCase):
    def test_regression_counts_arithmetic(self):
        c = V.regression_counts()
        self.assertEqual(c, {"filesystem": 9, "ambiguous": 6, "multidomain": 8, "rare": 6, "wrong_first_tool": 6, "exec_failure": 6,
                             "denied": 6, "approval": 6, "injection_workspace": 5, "injection_tool": 4, "unknown_tool": 4, "sequential": 6})
        self.assertEqual(sum(c.values()), 72)
        self.assertNotIn("expansion", c)
        # 0.2x non-expansion total is 96 and 72 / 96 = 3 / 4
        self.assertEqual(sum(n for k, n in V.HELDOUT_CATEGORY_COUNTS.items() if k != "expansion"), 96)

    def test_regression_counts_other_totals_sum(self):
        for total in (60, 72, 80):
            self.assertEqual(sum(V.regression_counts(total=total).values()), total)


# ------------------------------------------------------------------------------- a synthetic 120-task corpus

def build_synthetic(root: Path, mutate=None):
    """A complete synthetic heldout-0.2y tree: 0.2x fixtures/stores/stubs, 72 regression tasks cut from the 0.2x
    tasks, 48 discovery_needed tasks built from expansion-117, and the log that produced them."""
    x = BENCH / "heldout-0.2x"
    for d in ("fixtures", "stores"):
        (root / d).mkdir()
    shutil.copytree(x / "fixtures" / "beacon", root / "fixtures" / "beacon")
    shutil.copy(x / "stores" / "beacon.json", root / "stores" / "beacon.json")
    (root / "stubs").mkdir()
    src = [json.loads(p.read_text(encoding="utf-8")) for p in sorted((x / "tasks").glob("*.json"))]
    table = A.gen_slots()
    A.assemble(table, [{"slot": s["id"], "request": "Please help: " + " ".join(s["must_contain"]) + " " + s["need"]} for s in table["slots"]], root)
    entries = decide(make_entries(nonfile=36, default=36))
    for e in entries:
        e["fixture"] = "beacon"
    tmpl = next(t for t in src if t["id"] == "expansion-117")
    k = 0
    for e in entries:
        if e["status"] != "accepted":
            continue
        k += 1
        t = copy.deepcopy(tmpl)
        t["id"] = f"ambiguous-{100 + k:03d}"
        t["category"] = "ambiguous"
        t["user_request"] = e["request"]
        t["requested_domains"] = L.derive_match(e["request"], RULE)["domains"]
        t["judge"]["checks"] = [c for c in t["judge"]["checks"] if c["kind"] != "expansion_occurred"]
        t["notes"] = f"heldout-0.2y; template: syn-d{k}; kind: discovery_needed; group: {e['group']}; request: {e['id']}"
        (root / "tasks" / f"{t['id']}.json").write_text(json.dumps(t), encoding="utf-8")
    log = root.parent / "authoring-log.jsonl"
    log.write_text("".join(json.dumps(e, sort_keys=True) + "\n" for e in entries), encoding="utf-8")
    if mutate:
        mutate(root, entries)
    return log


def synth_errors(root: Path, log: Path, calibration=False):
    cfg = {"log": log, "log_set": "target"}
    ctx = {"y": True, "rule": RULE, "catalog": set(CATALOG), "tdom": TDOM, "prompt_sha": V.file_sha(BENCH / "prompts" / "system.md"),
           "stores": root / "stores", "root": root}
    schema = json.loads(V.SCHEMA_PATH.read_text(encoding="utf-8"))
    errs, tasks = [], []
    for p in sorted((root / "tasks").glob("*.json")):
        t = json.loads(p.read_text(encoding="utf-8"))
        es = V.schema_errors(schema, t, schema) or V.task_errors(t, p, ctx)
        errs += [f"{p.name}: {x}" for x in es]
        tasks.append(t)
    errs += V.backend_errors(tasks, ctx)
    cats = schema["$defs"]["Category"]["enum"]
    per = {c: V.Counter(t["split"] for t in tasks if t["category"] == c) for c in cats}
    errs += V.heldout_y_errors(tasks, per, ctx, cfg, calibration)
    return errs


class SyntheticCorpus(unittest.TestCase):
    def test_complete_corpus_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            log = build_synthetic(root)
            errs = synth_errors(root, log)
            self.assertEqual(errs, [])

    def test_leaky_discovery_request_fails_the_corpus(self):
        def leak(root, entries):
            p = root / "tasks" / "ambiguous-101.json"
            t = json.loads(p.read_text())
            t["user_request"] = "Use read_file on the rota"
            t["requested_domains"] = L.derive_match(t["user_request"], RULE)["domains"]
            p.write_text(json.dumps(t))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, leak))
            self.assertTrue(any("leakage lint rejects" in x for x in errs), errs)

    def test_group_counts_must_be_24_24(self):
        def drop(root, entries):  # turn one nonfile task into a default-style request: 23 / 25
            p = root / "tasks" / "ambiguous-101.json"
            t = json.loads(p.read_text())
            t["user_request"] = default_request(77)
            t["requested_domains"] = ["filesystem"]
            p.write_text(json.dumps(t))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, drop))
            self.assertTrue(any("derive group nonfile, expected exactly 24" in x for x in errs), errs)
            self.assertTrue(any("fewer than 24 discovery_needed tasks derive a non-file domain" in x for x in errs), errs)

    def test_wrong_regression_counts_fail(self):
        def remove(root, entries):
            (root / "tasks" / "filesystem-009.json").unlink()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, remove))
            self.assertTrue(any("regression counts" in x for x in errs), errs)
            self.assertTrue(any("119 tasks, expected exactly 120" in x for x in errs), errs)

    def test_task_not_in_log_fails(self):
        def swap(root, entries):
            p = root / "tasks" / "ambiguous-102.json"
            t = json.loads(p.read_text())
            t["notes"] = t["notes"].replace("request: q-", "request: q-9")
            p.write_text(json.dumps(t))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, swap))
            self.assertTrue(any("log accepted" in x for x in errs), errs)

    def test_discovery_task_with_required_capability_or_covered_nonfile_fails(self):
        def cap(root, entries):
            p = root / "tasks" / "ambiguous-101.json"
            t = json.loads(p.read_text())
            t["required_capabilities"] = ["read_file"]
            p.write_text(json.dumps(t))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, cap))
            self.assertTrue(any("must not require a capability" in x for x in errs), errs)

    def test_answer_absent_from_fixture_fails(self):
        def unsolvable(root, entries):
            p = root / "tasks" / "ambiguous-103.json"
            t = json.loads(p.read_text())
            t["judge"]["checks"][0]["values"] = ["Zebediah Quux"]
            p.write_text(json.dumps(t))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            errs = synth_errors(root, build_synthetic(root, unsolvable))
            self.assertTrue(any("does not occur in its fixture" in x for x in errs), errs)

    def test_missing_log_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "heldout-0.2y"
            root.mkdir()
            log = build_synthetic(root)
            log.unlink()
            errs = synth_errors(root, log)
            self.assertTrue(any("is missing" in x for x in errs) or any("authoring-log" in x for x in errs), errs)


if __name__ == "__main__":
    unittest.main(verbosity=1)
