"""Authoring driver for 0.2y attempt 2 (scratch; attempt 1 driver plus amendment 3). Phases: fixtures SET | requests SET.
Amendment 3: area-tagged facts (checked by authoring_02y.fact_errors), over-delivery logged as
discarded_over_delivery (facts stay unused), a call that returns nothing logged as one empty_call line."""
import json, os, re, subprocess, sys, time, hashlib
from pathlib import Path

WT = Path(os.environ.get("DRIVE_WT", "/home/drakestapleton/workspace/cand3-campaign/wt-ip-02y"))
BENCH = WT / "bench"
Y = BENCH / "heldout-0.2y"
IN = Y / "authoring-inputs"
SCR = Path(os.environ.get("DRIVE_SCR", "/home/drakestapleton/.claude/jobs/8a0dcc68/tmp/auth2"))
EMPTY = SCR / "cwd"
EMPTY.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(BENCH / "tools"))
import lint_02y as L, validate as V, authoring_02y as AU, assemble_02y as AS

MODEL = "gpt-6-sol"
CALLS = SCR / "calls.jsonl"
doc = (Y / "AUTHORING.md").read_text(encoding="utf-8")
sec4 = doc[doc.index("## 4. Verbatim briefs"):doc.index("## 5. Procedure")]
BLOCKS = re.findall(r"```text\n(.*?)```", sec4, flags=re.S)
B_FIX, B_E, B_Q, B_TOP, B_REG = BLOCKS[0], BLOCKS[1], BLOCKS[2], BLOCKS[3], BLOCKS[4]
assert "Write {N} separate" in B_FIX or "{N} separate" in B_FIX
assert "Some earlier requests" in B_TOP and "Below are {N}" in B_REG


def root_of(setname):
    return Y if setname == "target" else Y / "calibration"


def call(name, prompt):
    """One codex call; raw prompt and output kept; returns stdout text."""
    d = IN / name.split("/")[0]
    d.mkdir(parents=True, exist_ok=True)
    (IN / f"{name}.prompt.txt").parent.mkdir(parents=True, exist_ok=True)
    (IN / f"{name}.prompt.txt").write_text(prompt, encoding="utf-8")
    cmd = ["codex", "exec", "--skip-git-repo-check", "-c", f'model="{MODEL}"', prompt]
    t0 = time.time()
    p = subprocess.run(cmd, cwd=EMPTY, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=3600)
    (IN / f"{name}.out.txt").write_text(p.stdout, encoding="utf-8")
    rec = {"call": name, "model": MODEL, "command": 'codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"', "rc": p.returncode,
           "seconds": int(time.time() - t0), "out_bytes": len(p.stdout), "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    with CALLS.open("a") as f:
        f.write(json.dumps(rec) + "\n")
    if p.returncode != 0:
        (IN / f"{name}.err.txt").write_text(p.stderr[-4000:], encoding="utf-8")
        raise SystemExit(f"codex failed rc={p.returncode} for {name}")
    return p.stdout


def parse_json(text):
    a, b = text.find("{"), text.rfind("}")
    return json.loads(text[a:b + 1])


def state(setname):
    p = SCR / f"state-{setname}.json"
    return json.loads(p.read_text()) if p.exists() else {}


def save(setname, st):
    (SCR / f"state-{setname}.json").write_text(json.dumps(st, indent=1))


def render_workspaces(wss, facts_by_ws):
    out = []
    for ws in wss:
        out.append(f"{ws['slug']}: {ws['description']}")
        for f in facts_by_ws.get(ws["slug"], []):
            out.append(f"- {f['id']}: {f['fact']}")
        out.append("")
    return "\n".join(out).rstrip()


def fixtures(setname):
    n = AU.workspaces_needed()
    assert n == 6
    prompt = B_FIX.replace("{N}", str(n))
    attempt = 0
    while True:
        attempt += 1
        name = f"{setname}/fixtures-call{attempt}"
        out = call(name, prompt)
        try:
            data = parse_json(out)
            wss = data["workspaces"]
            errs = []
            if len(wss) != n:
                errs.append(f"{len(wss)} workspaces, expected {n}")
            for ws in wss:
                if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{1,40}", ws.get("slug", "")):
                    errs.append(f"bad slug {ws.get('slug')!r}")
                for rel in ws.get("files", {}):
                    if rel.startswith("/") or ".." in rel.split("/"):
                        errs.append(f"{ws['slug']}: bad path {rel}")
                if len(ws.get("facts", [])) < 40:
                    errs.append(f"{ws['slug']}: only {len(ws.get('facts', []))} facts")
            errs += AU.fact_errors(wss)
        except Exception as e:
            errs = [f"unparseable: {e}"]
        if not errs and len({w["slug"] for w in wss}) == len(wss):
            break
        print("fixture call failed checks:", errs[:8])
        (IN / f"{name}.rejected.txt").write_text("\n".join(errs), encoding="utf-8")
        if attempt >= 3:
            raise SystemExit("fixtures failed 3 times; stop and report")
    root = root_of(setname)
    for ws in wss:
        for rel, text in ws["files"].items():
            p = root / "fixtures" / ws["slug"] / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text, encoding="utf-8")
    pc = AU.pool_check(wss)
    print("pool", pc)
    st = state(setname)
    st["workspaces"] = [{"slug": w["slug"], "description": w["description"], "facts": w["facts"], "files": sorted(w["files"])} for w in wss]
    st["split"] = AU.split_record(wss)
    st["pool"] = pc
    st["fixture_attempts"] = attempt
    save(setname, st)
    (IN / setname).mkdir(parents=True, exist_ok=True)
    (IN / setname / "fact-split.json").write_text(json.dumps(st["split"], indent=1), encoding="utf-8")
    (IN / setname / "facts.json").write_text(json.dumps([{"slug": w["slug"], "facts": w["facts"]} for w in wss], indent=1), encoding="utf-8")
    if not pc["ok"]:
        raise SystemExit(f"pool short: {pc}")


def requests(setname):
    st = state(setname)
    wss = st["workspaces"]
    rule = V.load_rule()
    rec = json.loads(V.CATALOG_PATH.read_text(encoding="utf-8"))
    catalog = {c["name"] for c in rec["capabilities"]}
    tdom = V.tool_domains()
    root = root_of(setname)
    files = {w["slug"]: V._fixture_files(root / "fixtures", w["slug"]) for w in wss}
    all_facts = AU.with_ids(wss)
    briefs = {"everyday": B_E, "question": B_Q}
    log = st.get("log", [])
    used = {b: set() for b in AU.BRIEFS}
    for e in log:
        if e["status"] not in ("discarded_over_delivery", "empty_call"):
            used[e["brief"]].add(e["fact_id"])
    msgs = {b: 0 for b in AU.BRIEFS}
    for e in log:
        if e["status"] == "discarded_lint":
            msgs[e["brief"]] += 1
    target = {"nonfile": 24, "default": 24} if setname == "target" else {"any": 30}
    counts = {g: 0 for g in target}
    for e in log:
        if e["status"] == "accepted":
            counts["any" if setname != "target" else e["group"]] += 1
    rnd = st.get("round", -1) + 1
    while True:
        if rnd == 0:
            plan = [("everyday", 36), ("question", 36)]
        else:
            short = [g for g in target if counts[g] < target[g]]
            if not short:
                break
            if rnd > 3:
                st["stopped"] = f"groups still short after 3 top-ups: {counts}"
                save(setname, st)
                raise SystemExit(st["stopped"])
            if setname == "target":
                plan = [({"nonfile": "everyday", "default": "question"}[g], 12 * rnd) for g in short]
            else:
                plan = [("everyday", 12 * rnd), ("question", 12 * rnd)]
        for brief, n in plan:
            subset = AU.facts_for_author(wss, brief, used[brief])
            fb = {}
            for f in subset:
                fb.setdefault(f["id"].rsplit("-", 1)[0], []).append(f)
            wtext = render_workspaces(wss, fb)
            text = briefs[brief].replace("{N_WORKSPACES}", str(len(wss))).replace("{WORKSPACES}", wtext).replace("{N}", str(n))
            if rnd >= 1:
                para = B_TOP.replace("{K}", str(n)).replace("{MESSAGES}", "\n".join(["- " + L.GENERIC_MESSAGE] * max(1, msgs[brief])))
                marker = "Answer with one JSON object"
                text = text.replace(marker, para.strip() + "\n\n" + marker, 1)
                text = text.replace("Write " + str(n) + " requests", "Write " + str(n) + " requests", 1)
            name = f"{setname}/requests-r{rnd}-{brief}"
            out = call(name, text)
            reqs = parse_json(out)["requests"]
            if len(reqs) != n:
                print(f"NOTE: {name} returned {len(reqs)} requests, asked {n}")
            valid = {f["id"] for f in AU.with_ids(wss) if AU.brief_of(f) == brief}
            if not reqs:  # amendment 3: the round stays visible
                log.append({"set": setname, "id": f"q-{len(log) + 1:04d}", "round": rnd, "brief": brief,
                            "fixture": None, "request": "", "status": "empty_call"})
            # amendment 3: requests beyond the ask are logged after the counted ones, never linted, fact unused
            over_entries = [{"set": setname, "id": None, "round": rnd, "brief": brief, "fixture": r.get("workspace"),
                             "request": r.get("request"), "answer": r.get("answer"), "template": r.get("template"),
                             "fact_id": r.get("fact_id"), "status": "discarded_over_delivery"} for r in reqs[n:]]
            for r in reqs[:n]:
                qid = f"q-{len(log) + 1:04d}"
                ws = r.get("workspace")
                res = L.lint_request(r["request"], catalog, rule, files.get(ws, ()), tdom)
                key = "any" if setname != "target" else res["group"]
                if not res["ok"]:
                    status = "discarded_lint"
                    msgs[brief] += 1
                elif key in counts and counts[key] < target[key]:
                    status = "accepted"
                    counts[key] += 1
                else:
                    status = "discarded_surplus"
                if ws not in files:
                    raise SystemExit(f"{qid}: unknown workspace {ws!r}")
                fid = r.get("fact_id")
                log.append({"set": setname, "id": qid, "round": rnd, "brief": brief, "fixture": ws, "request": r["request"],
                            "answer": r.get("answer"), "template": r.get("template"), "fact_id": fid,
                            "fact_in_brief_subset": fid in valid, "codes": res["codes"], "lint": res["messages"],
                            "detail": res["detail"], "group": res["group"], "status": status})
                used[brief].add(fid)
            for e in over_entries:  # after the counted ones, so ids follow the author's array order
                e["id"] = f"q-{len(log) + 1:04d}"
                log.append(e)
        st["log"] = log
        st["round"] = rnd
        save(setname, st)
        print(f"round {rnd}: counts {counts}, log {len(log)}")
        rnd += 1
    st["counts"] = counts
    save(setname, st)


def main():
    ph = sys.argv[1]
    if ph == "fixtures":
        fixtures(sys.argv[2])
    elif ph == "requests":
        requests(sys.argv[2])
main()
