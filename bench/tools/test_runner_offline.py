"""Offline test of the paired runner against a scripted fake OpenAI-compatible endpoint.

No GPU, no model, no qual task: synthetic tasks (derived from dev tasks) exercise the real code
path (Lenshift -> Core -> CrossAxis -> Crossveil with the real OdysseusAuthority): an excluded
tool call that triggers expansion, a discovery call, an injected fault with retry, and an unknown
tool name, and (``--backends sim-1``) every simulated backend kind, twice, with identical results.
Needs the Odysseus venv and ODYSSEUS_SRC (not run in the stdlib-only CI job):

  ODYSSEUS_SRC=<odysseus@2992bf6> <venv>/bin/python bench/tools/test_runner_offline.py
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
    if user.startswith("UNKNOWN"):
        if n_tool == 0:
            return None, [call(0, "summarize_repo", {})]
        return "summarize_repo is not available", None
    raise AssertionError("unscripted request")


class Handler(BaseHTTPRequestHandler):
    log = []

    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        tools = body.get("tools") or []
        prompt = 50 + 7 * len(tools) + 3 * len(body["messages"])
        if body.get("max_tokens") == 1:
            msg = {"role": "assistant", "content": "x"}
        else:
            text, calls = script(body["messages"], tools)
            msg = {"role": "assistant", "content": text or ""}
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
        assert a["metrics"]["exposed_tools_first"] == 71 and a["metrics"]["expansions"] == 0
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
        # resume: nothing is rerun
        before = len(Handler.log)
        run_bench.main(["--tasks", "dev", "--condition", "both", "--endpoint", f"http://127.0.0.1:{srv.server_port}/v1",
                        "--out", str(out), "--tasks-dir", str(tdir), "--allow-nonfrozen", "--no-warmup", "--resume"])
        assert len(Handler.log) == before
    print("offline runner test: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
