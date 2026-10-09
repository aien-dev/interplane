"""Offline test of the paired runner against a scripted fake OpenAI-compatible endpoint.

No GPU, no model, no qual task: synthetic tasks (derived from dev tasks) exercise the real code
path (Lenshift -> Core -> CrossAxis -> Crossveil with the real OdysseusAuthority): an excluded
tool call that triggers expansion, a discovery call, an injected fault with retry, and an unknown
tool name, and (``--backends sim-1``) every simulated backend kind, twice, with identical results.
``--expansion-policy runtime-v1`` (PROTOCOL-0.2x.md section 5): triggers T1 and T2, the duplicate
rule, the cap-exhausted note, and the negative controls N1 (denied) and N2 (approval pending).
Needs the Odysseus venv and ODYSSEUS_SRC (CI job adapter-odysseus runs it):

  ODYSSEUS_SRC=<odysseus@a8c147b> <venv>/bin/python bench/tools/test_runner_offline.py
"""

import copy
import json
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
sys.path.insert(0, str(HERE))
import run_bench  # noqa: E402

# The runner renders the adapter's whole recorded catalog (71 tools at 2992bf6, 88 at a8c147b).
N_TOOLS = len(run_bench.catalog_with_domains().capabilities)


SIM_ARGS = {
    "bash": {"command": "ls"},
    "write_file": {"path": "OUT.txt", "content": "v1\n"},
    "ask_user": {"question": "Which one?", "options": [{"label": "a"}, {"label": "b"}]},
    "manage_notes": {"action": "search", "query": "palette"},
    "get_workspace": {},
}


def call(i, name, args):
    return {"id": f"call_{i}", "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}


def script(messages, tools):
    user = messages[1]["content"]
    n_tool = sum(1 for m in messages if m["role"] == "tool")
    names = {t["function"]["name"] for t in tools}
    if user.startswith("EXPAND"):
        if n_tool == 0:
            return None, [call(0, "read_file", {"path": "release-checklist.md"})]
        return "2026-11-14", None
    if user.startswith("DISCO"):
        if n_tool == 0:
            if run_bench.DISCOVERY in names:
                return None, [call(0, run_bench.DISCOVERY, {"query": "serve preset"})]
            return None, [call(0, "list_serve_presets", {})]
        if n_tool == 1 and run_bench.DISCOVERY in names and "list_serve_presets" in names:
            return None, [call(1, "list_serve_presets", {})]
        return "tide-small, tide-large, whisper-batch", None
    if user.startswith("FAULT"):
        if n_tool < 2:
            return None, [call(n_tool, "read_file", {"path": "config/app.toml"})]
        return "port 8417", None
    if user.startswith("SIM1 "):
        # one call per run: a private or workspace read, or a shell result, arms Odysseus's gate
        if n_tool == 0:
            tool = user.split()[1]
            return None, [call(0, tool, SIM_ARGS[tool])]
        return "Ada Quill", None
    if user.startswith("RT-T2"):  # an unexposed catalog tool, once
        if n_tool == 0:
            return None, [call(0, "read_file", {"path": "README.md"})]
        return "lanternfish", None
    if user.startswith("RT-T1"):  # an unknown name twice: the second is a duplicate trigger
        if n_tool < 2:
            return None, [call(n_tool, "workspace_summary", {})]
        return "workspace_summary is not available", None
    if user.startswith("RT-CAP"):  # three unexposed tools: the third exceeds max_expansions
        # tools with no backend fail with execution_error (N5): an unexposed call still fires T2
        seq = ["list_serve_presets", "list_cookbook_servers", "list_downloads"]
        if n_tool < 3:
            return None, [call(n_tool, seq[n_tool], {})]
        Handler.tool_contents.append([m.get("content") for m in messages if m["role"] == "tool"])
        return "done", None
    if user.startswith("RT-N1"):  # non-admin calls an unexposed admin tool: denied, never expanded
        if n_tool == 0:
            return None, [call(0, "manage_tokens", {"action": "list"})]
        return "denied", None
    if user.startswith("RT-N2"):  # a workspace read arms the gate; the unexposed send is held for approval
        if n_tool == 0:
            return None, [call(0, "read_file", {"path": "README.md"})]
        if n_tool == 1:
            return None, [call(1, "send_email", {"to": "a@example.org", "subject": "s", "body": "b"})]
        return "needs approval", None
    if user.startswith("UNKNOWN"):
        if n_tool == 0:
            return None, [call(0, "summarize_repo", {})]
        return "summarize_repo is not available", None
    if Handler.lenient:  # real held-out and dev tasks in the corpus-switch test: any answer will do
        return "ok", None
    raise AssertionError("unscripted request")


class Handler(BaseHTTPRequestHandler):
    log = []
    tool_contents = []
    seen = []  # every request body's switch fields, including the max_tokens=1 measurement requests
    lenient = False
    fake_reasoning = False   # reply with a reasoning field unless the request turned reasoning off
    force_reasoning = False  # ... even when it did (an invalid arm 4 run)

    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        tools = body.get("tools") or []
        off = body.get("reasoning_effort") == "none" or (body.get("chat_template_kwargs") or {}).get("enable_thinking") is False
        Handler.seen.append({"system": body["messages"][0]["content"], "reasoning_effort": body.get("reasoning_effort"),
                             "chat_template_kwargs": body.get("chat_template_kwargs"), "seed": body.get("seed"),
                             "max_tokens": body.get("max_tokens")})
        prompt = 50 + 7 * len(tools) + 3 * len(body["messages"])
        if body.get("max_tokens") == 1:
            msg = {"role": "assistant", "content": "x"}
        else:
            text, calls = script(body["messages"], tools)
            msg = {"role": "assistant", "content": text or ""}
            if Handler.fake_reasoning and (not off or Handler.force_reasoning):
                msg["reasoning"] = "thinking"
            if calls:
                msg["tool_calls"] = calls
            Handler.log.append((body["messages"][1]["content"][:6], [t["function"]["name"] for t in tools]))
        out = {"choices": [{"index": 0, "message": msg, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": prompt, "completion_tokens": 5}}
        data = json.dumps(out).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def task_from(tid, template, user, **over):
    t = json.loads((BENCH / "tasks" / f"{template}.json").read_text())
    t["id"], t["user_request"] = tid, user
    t.update(over)
    return t


def main() -> int:
    srv = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory() as td:
        tdir, out = Path(td) / "tasks", Path(td) / "run"
        tdir.mkdir()
        tasks = [
            task_from("expansion-901", "expansion-001", "EXPAND checklist", split="dev"),
            task_from("rare-901", "rare-001", "DISCO presets", split="dev", requested_domains=["filesystem"],
                      judge={"expected_answer": "x", "checks": [{"kind": "answer_contains_all", "values": ["tide-small"]}, {"kind": "expansion_occurred", "conditions": ["B"]}]}),
            task_from("exec_failure-901", "exec_failure-001", "FAULT config", split="dev"),
            task_from("unknown_tool-901", "unknown_tool-001", "UNKNOWN tool", split="dev"),
        ]
        tasks[1]["required_capabilities"] = ["list_serve_presets"]
        tasks[1]["allowed_alternatives"] = []
        for t in tasks:
            (tdir / f"{t['id']}.json").write_text(json.dumps(t))
        rc = run_bench.main(["--tasks", "dev", "--condition", "both", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                             "--out", str(out), "--tasks-dir", str(tdir), "--allow-nonfrozen", "--no-warmup"])
        assert rc == 0
        R = lambda tid, c: json.loads((out / "receipts" / f"{tid}.{c}.json").read_text())

        a, b = R("expansion-901", "A"), R("expansion-901", "B")
        assert a["metrics"]["exposed_tools_first"] == N_TOOLS and a["metrics"]["expansions"] == 0
        assert b["metrics"]["exposed_tools_first"] < 15, b["metrics"]["exposed_tools_first"]
        assert b["metrics"]["unexposed_calls"] == 1, b["metrics"]
        assert b["metrics"]["effective_expansions"] == 1 and b["judge"]["success"], b["judge"]
        assert "read_file" not in b["rounds"][0]["exposed_names"] and "read_file" in b["rounds"][1]["exposed_names"]
        assert b["metrics"]["missing_required_first"] is True and b["metrics"]["missing_required_final"] is False
        assert b["metrics"]["covered_after_effective_expansions"] == 1
        # visibility is not authority: the call was decided and executed by Odysseus in the same round
        assert b["calls"][0]["decision"] == "authorized" and b["calls"][0]["executed"] is True
        assert a["judge"]["success"] is True  # the expansion check applies to B only
        d_a, d_b = R("rare-901", "A"), R("rare-901", "B")
        assert d_b["metrics"]["discovery_calls"] == 1 and d_b["metrics"]["effective_expansions"] == 1, d_b["metrics"]
        assert d_b["calls"][-1]["synthetic"] == "stub" and d_b["calls"][-1]["decision"] == "authorized"
        assert d_b["judge"]["success"], d_b["judge"]
        assert d_a["metrics"]["discovery_calls"] == 0
        f_a = R("exec_failure-901", "A")
        assert f_a["metrics"]["execution_errors"] == 1 and f_a["metrics"]["recovered_after_error"] is True
        assert f_a["judge"]["success"], f_a["judge"]
        assert f_a["calls"][0]["synthetic"] == "fault" and f_a["calls"][1]["synthetic"] is None
        u_b = R("unknown_tool-901", "B")
        assert u_b["metrics"]["unknown_calls"] == 1 and u_b["calls"][0]["capability"] is None
        # token accounting: unit-pure and per round
        assert b["measure"]["tokens"]["unit"] == "tokens_model_reported"
        assert len(b["tool_schema_tokens_per_round"]) == len(b["rounds"])
        # judge reproduces from the stored transcript
        import bench_eval
        for tid in ("expansion-901", "rare-901", "exec_failure-901", "unknown_tool-901"):
            for c in "AB":
                r = R(tid, c)
                assert bench_eval.judge(json.loads((tdir / f"{tid}.json").read_text()), c, r["transcript"]) == r["judge"]
        # sim-1 backends: every backend kind executes after Odysseus authorizes; same results twice
        sdir = Path(td) / "simtasks"
        sdir.mkdir()
        sim_ids = {}
        for k, tool in enumerate(SIM_ARGS):
            tid = f"ambiguous-{910 + k}"
            sim_ids[tool] = tid
            (sdir / f"{tid}.json").write_text(json.dumps(task_from(tid, "ambiguous-004", f"SIM1 {tool}", split="dev")))
        expect = {"bash": "sim_declared_failure", "write_file": "sim", "ask_user": "sim", "manage_notes": "sim",
                  "get_workspace": "sim_handler"}
        runs = []
        for k in (1, 2):
            sout = Path(td) / f"simrun{k}"
            rc = run_bench.main(["--tasks", "dev", "--condition", "A", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                                 "--out", str(sout), "--tasks-dir", str(sdir), "--allow-nonfrozen", "--no-warmup",
                                 "--backends", "sim-1"])
            assert rc == 0
            m = json.loads((sout / "manifest.json").read_text())
            assert m["backends"]["mode"] == "sim-1" and m["deterministic_identity"]["backends"]["registry_sha256"]
            seen = {}
            for tool, tid in sim_ids.items():
                r = json.loads((sout / "receipts" / f"{tid}.A.json").read_text())
                c = r["calls"][0]
                assert c["tool"] == tool and c["decision"] == "authorized", c
                assert c["synthetic"] == expect[tool], (tool, c["synthetic"])
                if tool == "bash":
                    assert c["error_code"] == "execution_error"
                else:
                    assert c["status"] == "ok", (tool, c)
                assert r["judge"]["success"], r["judge"]
                seen[tool] = (c["status"], c["result_digest"])
            runs.append(seen)
        assert runs[0] == runs[1], runs
        # the reference backends (0.2 default) still refuse the same calls as "not executed"
        rout = Path(td) / "refrun"
        run_bench.main(["--tasks", "dev", "--condition", "A", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                        "--out", str(rout), "--tasks-dir", str(sdir), "--allow-nonfrozen", "--no-warmup", "--only", sim_ids["ask_user"]])
        c = json.loads((rout / "receipts" / f"{sim_ids['ask_user']}.A.json").read_text())["calls"][0]
        assert c["synthetic"] is None and c["error_code"] == "execution_error", c
        # runtime-v1 expansion policy (arm 3), condition B only, sim-1 backends
        rdir = Path(td) / "rttasks"
        rdir.mkdir()
        rt_tasks = {
            "RT-T2": task_from("ambiguous-920", "ambiguous-004", "RT-T2 calendar question", split="dev", requested_domains=["calendar"]),
            "RT-T1": task_from("ambiguous-921", "ambiguous-004", "RT-T1 calendar question", split="dev", requested_domains=["calendar"]),
            "RT-CAP": task_from("ambiguous-922", "ambiguous-004", "RT-CAP calendar question", split="dev", requested_domains=["calendar"]),
            "RT-N1": task_from("ambiguous-923", "ambiguous-004", "RT-N1 calendar question", split="dev", requested_domains=["calendar"],
                               authority_profile={"admin": False, "delegated_credential": False}),
            "RT-N2": task_from("ambiguous-924", "ambiguous-004", "RT-N2 file question", split="dev", requested_domains=["filesystem"]),
        }
        for t in rt_tasks.values():
            (rdir / f"{t['id']}.json").write_text(json.dumps(t))
        rtout = Path(td) / "rtrun"
        rc = run_bench.main(["--tasks", "dev", "--condition", "B", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                             "--out", str(rtout), "--tasks-dir", str(rdir), "--allow-nonfrozen", "--no-warmup",
                             "--backends", "sim-1", "--expansion-policy", "runtime-v1"])
        assert rc == 0
        m = json.loads((rtout / "manifest.json").read_text())
        assert m["expansion_policy"]["name"] == "runtime-v1", m["expansion_policy"]
        RT = lambda k: json.loads((rtout / "receipts" / f"{rt_tasks[k]['id']}.B.json").read_text())
        r = RT("RT-T2")
        (ev,) = r["expansions"]
        assert ev["trigger"] == "T2" and ev["added"] == ["read_file"] and ev["reason"] == "requested_excluded", ev
        assert ev["previous_selection_digest"] != ev["selection_digest"] and "read_file" not in ev["previous_exposed"]
        assert ev["budget"]["expansions_left"] == 1 and ev["budget"]["schema_bytes_left"] > 0, ev["budget"]
        # N4: the triggering call went through the pipeline once and is never re-issued
        assert [c["tool"] for c in r["calls"]] == ["read_file"] and r["calls"][0]["executed"] is True
        r = RT("RT-T1")
        assert [t["outcome"] for t in r["runtime_triggers"]] == ["expanded", "duplicate_trigger"], r["runtime_triggers"]
        (ev,) = r["expansions"]
        assert ev["trigger"] == "T1" and ev["reason"] == "discovery_hit" and ev["added"] and ev["hits"], ev
        assert all(c["executed"] is False and c["error_code"] == "unknown_capability" for c in r["calls"])
        r = RT("RT-CAP")
        assert [e["trigger"] for e in r["expansions"]] == ["T2", "T2", "T2"], (r["runtime_triggers"], [(c["tool"], c["status"], c["error_code"], c["exposed"]) for c in r["calls"]])
        last = r["expansions"][-1]
        assert last["added"] == [] and {x["reason"] for x in last["refused"]} == {"max_expansions"} and last["exhausted_note"]
        assert any(run_bench.EXHAUSTED_NOTE in (c or "") for c in Handler.tool_contents[-1]), Handler.tool_contents[-1]
        assert r["metrics"]["effective_expansions"] == 2
        # N1: the denied call stays denied: no expansion, no execution
        r = RT("RT-N1")
        assert r["calls"][0]["status"] == "denied" and r["calls"][0]["executed"] is False
        assert r["expansions"] == [] and [t["outcome"] for t in r["runtime_triggers"]] == ["N1_policy_denied"]
        assert "manage_tokens" not in r["final_exposed_names"]
        # N2: the pending call stays pending: no expansion, no execution, no approval consumed
        r = RT("RT-N2")
        assert r["calls"][1]["tool"] == "send_email" and r["calls"][1]["status"] == "requires_approval", r["calls"]
        assert r["calls"][1]["executed"] is False
        assert r["expansions"] == [] and [t["outcome"] for t in r["runtime_triggers"]] == ["N2_approval_required"]
        assert "send_email" not in r["final_exposed_names"]
        # T3 and the schema budget, directly on the policy (no live call yields these codes today)
        cat = run_bench.catalog_with_domains()
        byn = {c.name: c for c in cat.capabilities}
        rbytes = lambda names: len(run_bench.jcs(run_bench.render(byn, names, "B")).encode("utf-8"))
        sel0 = run_bench.select(cat, ["calendar"], always_include=run_bench.ALWAYS_INCLUDE,
                                renderer=run_bench.openai_tools_renderer)[0]
        miss = {"error_code": "capability_not_found", "capability": "read_file", "tool": "read_file",
                "status": "not_found", "exposed": False}
        pol = run_bench.RuntimeExpansion(cat, byn, len(run_bench.jcs(run_bench.openai_tools_renderer(cat.capabilities)).encode("utf-8")), rbytes)
        sel1, ev, note = pol.on_call(miss, 1, 0, sel0)
        assert ev["trigger"] == "T3" and "read_file" in ev["added"] and len(ev["added"]) > 1 and not note, ev
        assert ev["budget"]["schema_bytes_after"] <= ev["budget"]["schema_bytes_limit"]
        tiny = run_bench.RuntimeExpansion(cat, byn, rbytes([e["name"] for e in sel0.selected]), rbytes)
        sel2, ev, note = tiny.on_call(miss, 1, 0, sel0)
        assert sel2 is sel0 and ev["added"] == [] and note, ev
        assert [x["name"] for x in ev["refused"] if x["reason"] == "schema_budget"][:1] == ["read_file"], ev["refused"]
        # judge reproduces from the stored transcript under runtime-v1 too
        for t in rt_tasks.values():
            rr = json.loads((rtout / "receipts" / f"{t['id']}.B.json").read_text())
            assert bench_eval.judge(t, "B", rr["transcript"]) == rr["judge"]
        # ---- 0.2x campaign switches (PROTOCOL-0.2x.md sections 4, 8, 10): the seven conditions, no model needed
        assert run_bench.rotated(list(run_bench.CAMPAIGN_CONDITIONS), 0) == ["A", "A2", "A4", "B1", "B2", "B3", "B4"]
        assert run_bench.rotated(list(run_bench.CAMPAIGN_CONDITIONS), 1)[:2] == ["A2", "A4"]
        assert run_bench.rotated(list(run_bench.CAMPAIGN_CONDITIONS), 9) == run_bench.rotated(list(run_bench.CAMPAIGN_CONDITIONS), 2)
        assert run_bench.rotated(["A", "B"], 0) == ["A", "B"] and run_bench.rotated(["A", "B"], 1) == ["B", "A"]  # the 0.2 rule
        sp = (BENCH / "prompts" / "system.md").read_text()
        add = (BENCH / "heldout-0.2x" / "prompts" / "discovery-addendum.md").read_text()
        sp2 = sp + "\n" + add  # system.md ends with a newline: exactly one blank line between them
        assert run_bench.text_sha(sp2) != run_bench.text_sha(sp)
        ep = f"http://127.0.0.1:{srv.server_port}/v1"
        cout = Path(td) / "camp"
        Handler.fake_reasoning = True
        rc = run_bench.main(["--tasks", "dev", "--condition", "all", "--endpoint", ep, "--out", str(cout), "--tasks-dir", str(rdir),
                             "--allow-nonfrozen", "--no-warmup", "--backends", "sim-1", "--seed", "43"])
        Handler.fake_reasoning = False
        assert rc == 0
        conds7 = list(run_bench.CAMPAIGN_CONDITIONS)
        C = lambda k, c: json.loads((cout / "receipts" / f"{rt_tasks[k]['id']}.{c}.json").read_text())
        cm = json.loads((cout / "manifest.json").read_text())
        assert cm["conditions"] == conds7 and sorted(cm["campaign"]["conditions"]) == sorted(conds7), cm["conditions"]
        assert cm["campaign"]["seed"] == 43 and cm["generation"]["seed"] == 43 and cm["campaign"]["reasoning_off_field"] == {"reasoning_effort": "none"}
        assert (cout / "gpu_at_start.txt").exists() and (cout / "gpu_at_end.txt").exists() and "valid" in cm["latency_validity"]
        for k in rt_tasks:
            for c in conds7:
                r = C(k, c)
                assert r["condition"] == c and r["condition_spec"]["id"] == c and r["generation"]["seed"] == 43
                assert all(isinstance(x["prompt_tokens"], int) and isinstance(x["completion_tokens"], int) for x in r["rounds"])
                assert len(r["tool_schema_tokens_per_round"]) == len(r["rounds"]) and r["base_context_tokens"] > 0
        # addendum: only A2 and B2 carry it; the digest is of the composed prompt
        for c in conds7:
            want = run_bench.text_sha(sp2) if c in ("A2", "B2") else run_bench.text_sha(sp)
            assert C("RT-T2", c)["system_prompt_digest"] == want, c
            assert C("RT-T2", c)["condition_spec"]["prompt_addendum"] is (c in ("A2", "B2"))
        # reasoning off: only A4 and B4 send the field, and then no round carries reasoning
        for c in conds7:
            r = C("RT-T2", c)
            if c in ("A4", "B4"):
                assert r["reasoning"] == {"off_requested": True, "field": "reasoning_effort", "rounds_with_reasoning": 0, "invalid_for_arm4": False}, r["reasoning"]
            else:
                assert r["reasoning"]["off_requested"] is False and r["reasoning"]["rounds_with_reasoning"] == len(r["rounds"]), (c, r["reasoning"])
        # family: A family renders the full catalog and has no discovery tool; B family does
        for c in ("A", "A2", "A4"):
            r = C("RT-T2", c)
            assert r["metrics"]["exposed_tools_first"] == N_TOOLS and r["discovery_tool"] is None and r["final_exposed_names"] is None
        for c in ("B1", "B2", "B3", "B4"):
            r = C("RT-T2", c)
            assert r["metrics"]["exposed_tools_first"] < 15 and r["discovery_tool"] == run_bench.DISCOVERY
        # policy by condition id: only B3 runs runtime-v1 (T1 on an unknown tool, N1 and N2 as trigger rules)
        for c in ("B1", "B2", "B4"):
            assert "runtime_triggers" not in C("RT-T1", c) and C("RT-T1", c)["expansions"] == [], c
            (ev,) = C("RT-T2", c)["expansions"]
            assert ev["trigger"] == "requested_excluded" and ev["added"] == ["read_file"], ev
            assert len(C("RT-N1", c)["expansions"]) == 1  # the 0.2 mechanism still exposes after a denied call
        r = C("RT-T1", "B3")
        assert [e["trigger"] for e in r["expansions"]] == ["T1"] and r["runtime_triggers"]
        for k in ("RT-N1", "RT-N2"):  # negative controls: no expansion, no execution
            r = C(k, "B3")
            assert r["expansions"] == [] and all(not c["executed"] for c in r["calls"] if c["decision"] in ("denied", "requires_approval"))
            assert any(c["decision"] in ("denied", "requires_approval") for c in r["calls"])
        # the switch fields reach the request body only when asked for
        tt = rdir / "one"
        tt.mkdir()
        (tt / f"{rt_tasks['RT-T2']['id']}.json").write_text(json.dumps(rt_tasks["RT-T2"]))
        def bodies(cond, *extra):
            Handler.seen.clear()
            o = Path(td) / f"sw-{cond}-{'-'.join(extra) or 'plain'}"
            assert run_bench.main(["--tasks", "dev", "--condition", cond, "--endpoint", ep, "--out", str(o), "--tasks-dir", str(tt),
                                   "--allow-nonfrozen", "--no-warmup", "--backends", "sim-1", *extra]) == 0
            return list(Handler.seen), o
        seen_a, _ = bodies("A")
        assert seen_a and all(b["reasoning_effort"] is None and b["chat_template_kwargs"] is None and b["seed"] == 42 for b in seen_a)
        seen4, _ = bodies("A4")
        assert any(b["max_tokens"] == 1 for b in seen4) and all(b["reasoning_effort"] == "none" for b in seen4), seen4
        seen4k, _ = bodies("B4", "--reasoning-field", "chat_template_kwargs")
        assert all(b["chat_template_kwargs"] == {"enable_thinking": False} and b["reasoning_effort"] is None for b in seen4k)
        seen2, _ = bodies("A2")
        assert all(b["system"] == sp2 for b in seen2 if b["max_tokens"] != 1)  # every model turn
        assert any(b["system"] == sp2 and b["max_tokens"] == 1 for b in seen2)  # and the token measurement
        Handler.fake_reasoning = Handler.force_reasoning = True
        _, o = bodies("B4", "--seed", "44")
        Handler.fake_reasoning = Handler.force_reasoning = False
        r = json.loads(next((o / "receipts").glob("*.B4.json")).read_text())
        assert r["reasoning"]["invalid_for_arm4"] is True and r["generation"]["seed"] == 44, r["reasoning"]  # reasoning present: invalid
        # refusals: the campaign needs sim-1, the condition fixes the policy, held-out seeds are 42 to 44
        for bad in (["--condition", "B1"], ["--condition", "B1", "--backends", "sim-1", "--expansion-policy", "runtime-v1"],
                    ["--corpus", "0.2x", "--backends", "sim-1", "--seed", "7"]):
            try:
                run_bench.main(["--tasks", "dev", "--endpoint", ep, "--out", str(Path(td) / "bad"), "--tasks-dir", str(tt), "--allow-nonfrozen" if "7" not in bad else "--no-warmup", *bad])
                raise AssertionError(f"accepted {bad}")
            except SystemExit as e:
                assert str(e) != "0" and e.code != 0, bad
        # corpus switch: the real held-out set, frozen digests checked (no --allow-nonfrozen), fixtures,
        # stubs and stores resolved under bench/heldout-0.2x, the dev warm-up taken from bench/tasks
        hids = sorted(p.stem for p in (BENCH / "heldout-0.2x" / "tasks").glob("*.json"))
        pick = [next(i for i in hids if i.startswith("expansion-")), next(i for i in hids if i.startswith("multidomain-"))]
        hout = Path(td) / "held"
        Handler.lenient = True
        rc = run_bench.main(["--corpus", "0.2x", "--condition", "all", "--backends", "sim-1", "--seed", "42", "--endpoint", ep,
                             "--out", str(hout), "--only", ",".join(pick)])
        Handler.lenient = False
        assert rc == 0
        hm = json.loads((hout / "manifest.json").read_text())
        assert hm["corpus"]["match"] is True and hm["tasks"] == "qual" and hm["task_ids"] == pick, hm["task_ids"]
        assert hm["stochastic_execution"]["warmup"]["task"] == "filesystem-001" and hm["stochastic_execution"]["warmup"]["condition"] == "B1"
        assert hm["backends"]["stores"] and all(p.endswith(".json") for p in hm["backends"]["stores"])
        for tid in pick:
            for c in conds7:
                r = json.loads((hout / "receipts" / f"{tid}.{c}.json").read_text())
                assert r["task"]["split"] == "qual" and r["fixture"]["name"].startswith("fixtures/") and r["fixture"]["digest"]
        assert hm["order_rule"].startswith("tasks in id order; canonical condition order")
        # ---- condition B5 (PREREG-0.2y F1) and the R1 receipt fields
        base5 = ["ask_user", "ls", "glob", "grep", "read_file"]
        assert run_bench.B5_ALWAYS_INCLUDE == base5 and run_bench.ALWAYS_INCLUDE == ["ask_user"]
        # existing conditions, order and rotation are untouched: B5 is explicit only
        assert run_bench.CAMPAIGN_CONDITIONS == ("A", "A2", "A4", "B1", "B2", "B3", "B4")
        assert "B5" in run_bench.CONDITION_IDS and "B5" not in run_bench.CAMPAIGN_CONDITIONS
        assert run_bench.always_include_for("B3") == ["ask_user"] and run_bench.always_include_for("B5") == base5
        assert [run_bench.rotated(list(run_bench.CAMPAIGN_CONDITIONS), k) for k in range(3)] == \
            [["A", "A2", "A4", "B1", "B2", "B3", "B4"], ["A2", "A4", "B1", "B2", "B3", "B4", "A"], ["A4", "B1", "B2", "B3", "B4", "A", "A2"]]
        assert "always_include" not in run_bench.condition_spec({"expansion_policy": "0.2"}, "B3")
        assert all(c in cm["campaign"]["conditions"] and "always_include" not in cm["campaign"]["conditions"][c] for c in conds7)  # all: identity has no base set
        assert "base_sets" not in cm["deterministic_identity"] and cm["deterministic_identity"]["expansion_policy"]["applies_to"] == ["B3"]
        b5dir = Path(td) / "b5tasks"
        b5dir.mkdir()
        b5t = [task_from("ambiguous-925", "ambiguous-004", "DISCO calendar question", split="dev", requested_domains=["calendar"],
                         judge={"expected_answer": "x", "checks": [{"kind": "answer_contains_all", "values": ["tide-small"]}]}),
               task_from("ambiguous-926", "ambiguous-004", "RT-T2 calendar question", split="dev", requested_domains=["calendar"])]
        for t in b5t:
            (b5dir / f"{t['id']}.json").write_text(json.dumps(t))

        def run_b5(cond, name, *extra):
            o = Path(td) / name
            assert run_bench.main(["--tasks", "dev", "--condition", cond, "--endpoint", ep, "--out", str(o), "--tasks-dir", str(b5dir),
                                   "--allow-nonfrozen", "--no-warmup", "--backends", "sim-1", *extra]) == 0
            return o, json.loads((o / "manifest.json").read_text())
        o5, m5 = run_b5("B5", "b5run")
        o3, m3 = run_b5("B3", "b3run")
        assert m5["conditions"] == ["B5"] and m5["campaign"]["conditions"]["B5"]["always_include"] == base5
        assert m5["deterministic_identity"]["base_sets"] == {"B5": base5} and m5["expansion_policy"]["applies_to"] == ["B5"]
        assert "base_sets" not in m3["deterministic_identity"]
        assert m3["deterministic_identity"]["identity_digest"] != m5["deterministic_identity"]["identity_digest"]
        for tid in ("ambiguous-925", "ambiguous-926"):
            r5 = json.loads((o5 / "receipts" / f"{tid}.B5.json").read_text())
            r3 = json.loads((o3 / "receipts" / f"{tid}.B3.json").read_text())
            assert r5["condition_spec"]["expansion_policy"] == "runtime-v1" and r5["condition_spec"]["always_include"] == base5
            assert "always_include" not in r3["condition_spec"]
            ex5, ex3 = r5["rounds"][0]["exposed_names"], r3["rounds"][0]["exposed_names"]
            assert set(base5) <= set(ex5) and r5["discovery_tool"] == run_bench.DISCOVERY, ex5
            assert not ({"ls", "glob", "grep"} & set(ex3)), ex3  # B3 keeps the ask_user-only base set
            assert set(ex3) < set(ex5)  # B5 is B3 plus the base set and nothing else
            assert r5["selection"]["round1"]["selected"] and set(base5) <= {e["name"] for e in r5["selection"]["round1"]["selected"]}
        # R1: full assistant text of every round, query of every discovery call; existing fields unchanged
        r5 = json.loads((o5 / "receipts" / "ambiguous-925.B5.json").read_text())
        assert r5["rounds"][-1]["text"] == "tide-small, tide-large, whisper-batch"
        assert all(isinstance(r["text"], str) and "text_digest" in r for r in r5["rounds"])
        dq = [c for c in r5["calls"] if c["discovery"]]
        assert dq and dq[0]["query"] == "serve preset" and "arguments_digest" in dq[0], r5["calls"]
        assert bench_eval.r1_valid(r5) and bench_eval.r1_problems(r5) == []
        for rr in (json.loads(p.read_text()) for p in (out / "receipts").glob("*.B.json")):  # the 0.2 path carries R1 too
            assert bench_eval.r1_valid(rr)
        # a receipt missing R1 fields is invalid (the analyzer drops it for B5)
        stripped = copy.deepcopy(r5)
        for r in stripped["rounds"]:
            del r["text"]
        for c in stripped["calls"]:
            c.pop("query", None)
        assert not bench_eval.r1_valid(stripped) and len(bench_eval.r1_problems(stripped)) == len(stripped["rounds"]) + len(dq)
        import analyze
        shutil_copy = Path(td) / "b5bad"
        (shutil_copy / "receipts").mkdir(parents=True)
        (shutil_copy / "manifest.json").write_text((o5 / "manifest.json").read_text())
        (shutil_copy / "receipts" / "ambiguous-925.B5.json").write_text(json.dumps(stripped))
        (shutil_copy / "receipts" / "ambiguous-926.B5.json").write_text((o5 / "receipts" / "ambiguous-926.B5.json").read_text())
        ld = analyze.load_run(shutil_copy, b5dir)
        assert [x["task"] for x in ld["r1_invalid"]] == ["ambiguous-925"] and ld["r1_invalid"][0]["condition"] == "B5"
        assert analyze.load_run(o5, b5dir)["r1_invalid"] == []
        # the analyzer refuses a run holding an invalid B5 receipt instead of scoring around it
        try:
            analyze.build(shutil_copy, b5dir, [], "")
            raise AssertionError("build scored a run with an invalid B5 receipt")
        except SystemExit as e:
            assert "without valid R1 fields" in str(e), e
        # no rounds is invalid; a non-string query is valid only as a recorded invalid_arguments call
        assert bench_eval.r1_problems({"rounds": [], "calls": []}) == ["no rounds recorded"]
        assert not bench_eval.r1_valid({k: v for k, v in r5.items() if k != "rounds"})
        nullq = copy.deepcopy(r5)
        nc = [c for c in nullq["calls"] if c["discovery"]][0]
        nc["query"] = None
        assert not bench_eval.r1_valid(nullq), "a null query on an ok call must be invalid"
        nc["status"], nc["error_code"] = "error", "invalid_arguments"
        assert bench_eval.r1_valid(nullq), "the model's own malformed call, recorded as such, is valid"
        # the base set is verified against the catalog: a name the catalog lacks refuses the run
        saved_sets = dict(run_bench.BASE_SETS)
        run_bench.BASE_SETS["B5"] = base5 + ["no_such_tool"]
        try:
            run_b5("B5", "b5bad")
            raise AssertionError("accepted a base set naming a tool the catalog lacks")
        except SystemExit as e:
            assert "no_such_tool" in str(e), e
        finally:
            run_bench.BASE_SETS.clear()
            run_bench.BASE_SETS.update(saved_sets)
        # resume: nothing is rerun
        before = len(Handler.log)
        run_bench.main(["--tasks", "dev", "--condition", "both", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                        "--out", str(out), "--tasks-dir", str(tdir), "--allow-nonfrozen", "--no-warmup", "--resume"])
        assert len(Handler.log) == before
    # receipt `attempt` = runs of that condition (REPORT-0.2x: both-failed pairs were labelled 3)
    saved = (run_bench.run_condition, run_bench.measure_run_tokens, run_bench.make_receipt, run_bench.write_json)
    seen = {}
    run_bench.run_condition = lambda cx, task, cond, rid: {"infra_failure": cond == "A", "infra_error": "HTTPError 500" if cond == "A" else None}
    run_bench.measure_run_tokens = lambda cx, run: None
    run_bench.make_receipt = lambda cx, task, cond, run, attempt, prior, pm, rid: seen.setdefault(cond, (attempt, len(prior)))
    run_bench.write_json = lambda path, obj: None
    try:
        for campaign, want in ((True, {"A": (2, 2), "B1": (1, 2)}), (False, {"A": (3, 3), "B1": (3, 3)})):
            seen.clear()
            run_bench.run_pair({"campaign_mode": campaign}, {"id": "t"}, ["A", "B1"], "r", Path("."), ["A", "B1"])
            assert seen == want, (campaign, seen)
    finally:
        run_bench.run_condition, run_bench.measure_run_tokens, run_bench.make_receipt, run_bench.write_json = saved
    print("offline runner test: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
