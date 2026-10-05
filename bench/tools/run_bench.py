"""Paired full-catalog (A) vs CrossAxis-selected (B) benchmark runner, INTERPLANE 0.2.

Generalizes examples/receipt/run_receipt.py to the 44-task corpus in bench/ under
bench/PROTOCOL-0.2.md. Every model turn goes through the INTERPLANE pipeline (Lenshift openai ->
Core admit -> CrossAxis map -> Crossveil decide/execute) with Odysseus's own gate deciding and
Odysseus's own handlers executing read_file/ls/glob/grep. Nothing here grants authority.

  condition A: all 71 catalog tools rendered every round.
  condition B: CrossAxis `domain_match` v1 over the task's requested_domains + always_include
               ask_user, plus ONE always-included read-only discovery tool
               (`interplane_capabilities_search{query}`; it grants nothing and executes nothing in
               the runtime), plus bounded evidence-based expansion (spec/CROSSAXIS.md).

Run (needs the Odysseus checkout at 2992bf6 and its venv, see adapters/odysseus/README.md):

  ODYSSEUS_SRC=<odysseus@2992bf6> <odysseus-venv>/bin/python bench/tools/run_bench.py \\
      --tasks dev --condition both --out bench/runs/pilot-<ts>/

Tool arguments are never written to receipts; only digests and key names. Receipts are written
atomically per pair; ``--resume`` skips pairs already complete. ``--prepare`` writes only the
manifest (identity) and exits, so it can be committed before the first model request.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
REPO = BENCH.parent
for _p in (REPO / "python", REPO / "adapters" / "odysseus", BENCH / "tools"):
    if str(_p) not in sys.path:
        sys.path.insert(0, str(_p))

import bench_eval  # noqa: E402
import sim_backends  # noqa: E402
from interplane.core import jcs, sha256_hex  # noqa: E402
from interplane.crossaxis import (  # noqa: E402
    ExpansionError,
    discover,
    expand,
    measure,
    openai_tools_renderer,
    select,
    selection_digest,
)
from interplane.crossveil import Pipeline, make_result  # noqa: E402
from interplane_adapter_odysseus import _odysseus  # noqa: E402
from interplane_adapter_odysseus.authority import EXECUTABLE, RUNTIME_ID, OdysseusAuthority  # noqa: E402
from interplane_adapter_odysseus.dialect import make_registry  # noqa: E402
from interplane_adapter_odysseus.domains import catalog_with_domains  # noqa: E402
from interplane_adapter_odysseus.mapping import mapping_table  # noqa: E402

RECEIPT_VERSION = "0.2.0"
DISCOVERY = bench_eval.DISCOVERY_TOOL
ALWAYS_INCLUDE = ["ask_user"]
FROZEN_PROMPT_SHA = "207dd449409bacd6260943a1d918087334d9601586c1e2f069bf9336aaf647f5"
ODYSSEUS_COMMIT = "2992bf6"
DISCOVERY_SPEC = {
    "type": "function",
    "function": {
        "name": DISCOVERY,
        "description": (
            "Search the names and descriptions of tools that are NOT currently listed. Returns "
            "matching tool names and makes them available on your next turn. Read-only: it runs "
            "nothing and grants no permission; every tool you then call is still checked."
        ),
        "parameters": {
            "type": "object",
            "properties": {"query": {"type": "string", "description": "A word or phrase to look for."}},
            "required": ["query"],
        },
    },
}


class InfraError(Exception):
    """HTTP error, connection failure, timeout or unusable response: not model behaviour."""


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def digest_of(value) -> str:
    return "sha256:" + sha256_hex(jcs(value).encode("utf-8"))


def text_sha(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def file_sha(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def tree_digest(root: Path) -> str:
    lines = ""
    for p in sorted(f for f in root.rglob("*") if f.is_file()):
        rel = p.relative_to(root).as_posix()
        lines += f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {rel}\n"
    return "sha256:" + hashlib.sha256(lines.encode("utf-8")).hexdigest()


def write_json(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)


# ------------------------------------------------------------------------------- backend client

class Client:
    def __init__(self, endpoint: str, model: str, timeout: float, seed: int, temperature: float, max_tokens: int):
        self.endpoint, self.model, self.timeout = endpoint.rstrip("/"), model, timeout
        self.seed, self.temperature, self.max_tokens = seed, temperature, max_tokens

    def chat(self, messages: list, tools, max_tokens=None) -> tuple:
        body = {
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "seed": self.seed,
            "max_tokens": self.max_tokens if max_tokens is None else max_tokens,
        }
        if tools is not None:
            body["tools"] = tools
        data = jcs(body).encode("utf-8")
        headers = {"Content-Type": "application/json"}
        key = os.environ.get("INTERPLANE_PROBE_API_KEY")
        if key:
            headers["Authorization"] = f"Bearer {key}"
        req = urllib.request.Request(self.endpoint + "/chat/completions", data=data, headers=headers)
        t0 = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read().decode("utf-8", "replace")
            out = json.loads(raw)
            if not isinstance(out, dict) or not out.get("choices") or "message" not in out["choices"][0]:
                raise InfraError("response has no choices[0].message")
        except (urllib.error.URLError, socket.timeout, TimeoutError, ConnectionError, json.JSONDecodeError, OSError) as err:
            raise InfraError(f"{type(err).__name__}: {err}") from err
        return out, int((time.monotonic() - t0) * 1000), "sha256:" + sha256_hex(data)


class LiveCounter:
    """TokenCounter (interplane.crossaxis protocol): model-reported usage.prompt_tokens at max_tokens=1."""

    unit = "tokens_model_reported"
    source = "backend usage.prompt_tokens of a max_tokens=1 request (openai-compatible endpoint)"

    def __init__(self, client: Client):
        self.client, self.cache, self.requests = client, {}, 0

    def count_prompt(self, messages: list, tools) -> int:
        key = digest_of([messages, tools])
        if key not in self.cache:
            resp, _, _ = self.client.chat(messages, tools, max_tokens=1)
            self.requests += 1
            n = (resp.get("usage") or {}).get("prompt_tokens")
            if not isinstance(n, int):
                raise InfraError("backend reported no usage.prompt_tokens")
            self.cache[key] = n
        return self.cache[key]


# ------------------------------------------------------------------------------- authority

class BenchAuthority(OdysseusAuthority):
    """OdysseusAuthority plus the task's stubs and fault injections, applied only AFTER authorization.

    ``decide`` is inherited untouched, so Odysseus's gate always decides. ``execute`` is reached
    only for an authorized request; this subclass then returns the task's deterministic fault, or
    the task's stub for read-only tools the reference adapter does not execute, or (with
    ``--backends sim-1``) the simulated backend from ``bench/stubs/backends.json``, and feeds the
    result to Odysseus's own ``observe_tool_result`` exactly like a real result.
    """

    def __init__(self, workspace: str, task: dict, stubs: dict, sim=None, **kw):
        super().__init__(workspace, admin=task["authority_profile"]["admin"],
                         delegated_credential=task["authority_profile"]["delegated_credential"], **kw)
        self._stubs = stubs
        self._faults = task["fault_injection"]
        self._sim = sim  # sim_backends.SimSession, or None for the 0.2 reference backends
        self._auth_counts: Counter = Counter()
        self.bench_events: dict = {}  # request_id -> "fault" | "stub" | "sim" | "sim_handler" | "sim_declared_failure"

    def _fault_for(self, name: str, k: int):
        for f in self._faults:
            if f["capability"] == name and (f["on_calls"] == "all" or k in f["on_calls"]):
                return f
        return None

    def _synthetic(self, req, ctx, raw: dict, ok: bool):
        name, rid = req.capability, req.request_id
        block, message = self._block(name, req.arguments)
        if block is None:
            return self._error(rid, name, message or "odysseus rejected the tool call")
        trust = self._integrity(name, block.content)
        sc = self._context_for(str(ctx.get("trace_id")), {})
        sc.observe_tool_result(name, raw, block.content)
        if ok:
            return make_result(rid, "ok", runtime=RUNTIME_ID, capability=name, data=raw,
                               content_kind="tool_result", trust=trust, duration_ms=0)
        return make_result(rid, "error", runtime=RUNTIME_ID, capability=name, code="execution_error",
                           message=self._redact(str(raw["error"]))[:4096], content_kind="tool_result",
                           trust=trust, duration_ms=0)

    def execute(self, req, decision, ctx):
        name = req.capability
        self._auth_counts[name] += 1
        fault = self._fault_for(name, self._auth_counts[name])
        if fault is not None:
            self.bench_events[req.request_id] = "fault"
            return self._synthetic(req, ctx, {"error": fault["error"]}, ok=False)
        if name in self._stubs:
            self.bench_events[req.request_id] = "stub"
            return self._synthetic(req, ctx, copy.deepcopy(self._stubs[name]), ok=True)
        kind = self._sim.kind(name) if self._sim is not None else None
        if kind == "odysseus_handler":
            block, message = self._block(name, req.arguments)
            if block is None:
                return self._error(req.request_id, name, message or "odysseus rejected the tool call")
            try:
                raw = self._run_handler(name, block.content)
            except Exception as err:  # noqa: BLE001 - reported as an execution error, not raised
                raw = {"error": f"{name}: {type(err).__name__}", "exit_code": 1}
            if not isinstance(raw, dict):
                raw = {"error": f"{name}: handler returned no result", "exit_code": 1}
            self.bench_events[req.request_id] = "sim_handler"
            return self._synthetic(req, ctx, raw, ok=raw_ok(raw))
        if kind in ("computed", "declared_failure"):
            raw = self._sim.call(name, req.arguments)
            self.bench_events[req.request_id] = "sim" if kind == "computed" else "sim_declared_failure"
            return self._synthetic(req, ctx, raw, ok=raw_ok(raw))
        return super().execute(req, decision, ctx)


def raw_ok(raw: dict) -> bool:
    """Same success rule as OdysseusAuthority.execute: no error and exit code 0 or absent."""
    return not raw.get("error") and raw.get("exit_code") in (None, 0)


# ------------------------------------------------------------------------------- one run

def render(by_name: dict, names: list, cond: str) -> list:
    tools = openai_tools_renderer([by_name[n] for n in names])
    return tools + [DISCOVERY_SPEC] if cond == "B" else tools


def parse_args_of(tc) -> tuple:
    """(tool name, argument keys, arguments digest) of one raw tool call. Arguments are never kept."""
    fn = tc.get("function") if isinstance(tc, dict) else None
    name = fn.get("name") if isinstance(fn, dict) and isinstance(fn.get("name"), str) else None
    raw = fn.get("arguments") if isinstance(fn, dict) else None
    try:
        args = json.loads(raw) if isinstance(raw, str) else None
    except json.JSONDecodeError:
        args = None
    if isinstance(args, dict):
        return name, sorted(args), digest_of(args), args
    return name, [], text_sha(raw if isinstance(raw, str) else repr(raw)), None


def run_condition(cx: dict, task: dict, cond: str, run_id: str) -> dict:
    if cx["backends"] == "sim-1":
        # Fixed path, so get_workspace's text is identical on every run (runs are sequential).
        ws = os.path.join(tempfile.gettempdir(), "interplane-bench-ws", f"{task['id']}.{cond}")
        shutil.rmtree(ws, ignore_errors=True)
        os.makedirs(ws)
    else:
        ws = tempfile.mkdtemp(prefix="interplane-bench-")
    try:
        shutil.copytree(cx["bench"] / task["workspace_fixture"], ws, symlinks=True, dirs_exist_ok=True)
        return _run(cx, task, cond, run_id, ws)
    finally:
        shutil.rmtree(ws, ignore_errors=True)


def _run(cx: dict, task: dict, cond: str, run_id: str, ws: str) -> dict:
    client, catalog, by_name = cx["client"], cx["catalog"], cx["by_name"]
    stubs = {cap: json.loads((cx["bench"] / s["path"]).read_text(encoding="utf-8"))
             for cap, s in task["stub_results"].items()}
    sim = sim_backends.SimSession(ws, task["workspace_fixture"], cx["backend_registry"]) if cx["backends"] == "sim-1" else None
    authority = BenchAuthority(ws, task, stubs, sim=sim)
    if not authority.available:
        raise SystemExit("Odysseus is not importable: set ODYSSEUS_SRC")
    pipe = Pipeline(make_registry(), mapping_table(), authority)
    trace_id = f"bench-{run_id}-{task['id']}-{cond}"
    base_messages = [{"role": "system", "content": cx["system_prompt"]}, {"role": "user", "content": task["user_request"]}]
    messages = copy.deepcopy(base_messages)
    all_names = [c.name for c in catalog.capabilities]
    selection = None
    if cond == "B":
        selection, _ = select(catalog, task["requested_domains"], always_include=ALWAYS_INCLUDE,
                              renderer=openai_tools_renderer)

    def current_names() -> list:
        return all_names if selection is None else [e["name"] for e in selection.selected]

    rounds, calls, expansion_events, coverage_trace, round_tools = [], [], [], [], []
    final_answer, ended, infra_error = None, "max_rounds", None
    n_eff = 0
    t_start = time.monotonic()
    for turn in range(cx["max_rounds"]):
        names = current_names()
        names_set = set(names)
        tools = render(by_name, names, cond)
        round_tools.append(tools)
        coverage_trace.append({"round": turn + 1, "effective_expansions": n_eff, "covered": bench_eval.covered(task, names)})
        try:
            resp, ms, req_digest = client.chat(messages, tools)
        except InfraError as err:
            infra_error, ended = str(err), "infra_failure"
            round_tools.pop()
            coverage_trace.pop()
            break
        choice = resp["choices"][0]
        msg = choice["message"]
        usage = resp.get("usage") or {}
        orig = msg.get("tool_calls") if isinstance(msg.get("tool_calls"), list) else []
        disc_idx = [i for i, tc in enumerate(orig)
                    if cond == "B" and isinstance(tc, dict) and isinstance(tc.get("function"), dict)
                    and tc["function"].get("name") == DISCOVERY]
        filt_idx = [i for i in range(len(orig)) if i not in disc_idx]
        if disc_idx:
            pmsg = dict(msg)
            pmsg["tool_calls"] = [orig[i] for i in filt_idx]
        else:
            pmsg = msg
        out = None
        if not (disc_idx and not filt_idx):
            out = pipe.run_turn("openai", client.model, pmsg, trace_id, turn)
        outcome = out.outcome if out is not None else "tool_request"
        text = (out.text if out is not None else (msg.get("content") or "")) or ""
        rounds.append({
            "round": turn + 1,
            "request_digest": req_digest,
            "tools_digest": digest_of(tools),
            "tools_bytes": len(jcs(tools).encode("utf-8")),
            "exposed_tools_count": len(tools),
            "exposed_caps_count": len(names),
            "exposed_names": list(names) if cond == "B" else None,
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "latency_ms": ms,
            "finish_reason": choice.get("finish_reason"),
            "reasoning_present": bool(msg.get("reasoning") or msg.get("reasoning_content")),
            "outcome": outcome,
            "text_digest": text_sha(text) if text else None,
            "n_tool_calls": len(orig),
        })
        if out is not None and out.outcome != "tool_request":
            if out.outcome == "no_tool":
                final_answer, ended = text, "answer"
            else:
                final_answer, ended = None, "rejected_turn"
            break
        # ---- record the calls the pipeline decided
        tool_msgs: list = [None] * len(orig)
        recs: dict = {}
        if out is not None:
            for j, i in enumerate(filt_idx):
                res, rec = out.results[j], out.observed[j]
                name, keys, adig, _ = parse_args_of(orig[i])
                prov = res.provenance or {}
                cap = prov.get("capability")
                recs[i] = {
                    "round": turn + 1, "index": i, "tool": name, "capability": cap,
                    "argument_keys": keys, "arguments_digest": adig,
                    "request_id": rec.request_id, "decision": rec.decision, "status": res.status,
                    "error_code": res.error.code if res.error else None,
                    "executed": rec.execute_invoked, "result_digest": rec.result_digest,
                    "result_trust": prov.get("trust"), "result_content_kind": prov.get("content_kind"),
                    "exposed": (cap in names_set) if cap is not None else None,
                    "synthetic": authority.bench_events.get(rec.request_id),
                    "discovery": False,
                }
                tool_msgs[i] = out.rendered[j]
        # ---- expansion and discovery, in call order (visibility only; authority was already decided)
        for i in range(len(orig)):
            if i in recs:
                cap = recs[i]["capability"]
                if cond == "B" and cap is not None and cap in by_name and cap not in set(current_names()):
                    try:
                        selection = expand(selection, catalog, {"kind": "requested_excluded", "name": cap})
                        entry = selection.extra["expansions"][-1]
                        expansion_events.append({"round": turn + 1, "trigger": "requested_excluded", "call_index": i,
                                                 "selection_digest": selection.extra["selection_digest"], **entry})
                        n_eff += 1 if entry["added"] else 0
                    except ExpansionError as err:
                        expansion_events.append({"round": turn + 1, "trigger": "requested_excluded", "call_index": i,
                                                 "error": str(err), "added": [], "refused": []})
            elif i in disc_idx:
                name, keys, adig, args = parse_args_of(orig[i])
                q = args.get("query") if isinstance(args, dict) else None
                rid = (orig[i].get("id") if isinstance(orig[i], dict) else None) or f"{trace_id}:t{turn}:c{i}"
                rec = {"round": turn + 1, "index": i, "tool": DISCOVERY, "capability": None,
                       "argument_keys": keys, "arguments_digest": adig, "request_id": rid, "decision": None,
                       "status": "ok", "error_code": None, "executed": False, "result_digest": None,
                       "result_trust": "trusted_runtime", "result_content_kind": "discovery", "exposed": None,
                       "synthetic": None, "discovery": True}
                if not isinstance(q, str) or not q.strip():
                    content = {"error": "query must be a non-empty string"}
                    rec["status"], rec["error_code"] = "error", "invalid_arguments"
                else:
                    hits = discover(catalog, selection, q)
                    content = {"query": q, "matches": [{"name": n, "description": (by_name[n].description or "")[:100]} for n in hits]}
                    rec["discovery_hits"] = len(hits)
                    if hits:
                        try:
                            selection = expand(selection, catalog, {"kind": "discovery_hit", "query": q, "names": hits})
                            entry = selection.extra["expansions"][-1]
                            expansion_events.append({"round": turn + 1, "trigger": "discovery_hit", "call_index": i,
                                                     "selection_digest": selection.extra["selection_digest"], **entry})
                            n_eff += 1 if entry["added"] else 0
                            content["now_available"] = entry["added"]
                            if entry["refused"]:
                                content["not_added"] = entry["refused"]
                        except ExpansionError as err:
                            content["error"] = str(err)
                    else:
                        content["note"] = "no matching tools"
                body = jcs(content)
                rec["result_digest"] = text_sha(body)
                recs[i] = rec
                tool_msgs[i] = {"role": "tool", "tool_call_id": rid, "content": body}
        calls.extend(recs[i] for i in sorted(recs))
        messages.append({"role": "assistant", "content": msg.get("content") or "", "tool_calls": orig})
        messages.extend(m for m in tool_msgs if m is not None)
    wall_ms = int((time.monotonic() - t_start) * 1000)
    final_names = current_names()
    coverage_trace.append({"round": len(rounds) + 1, "effective_expansions": n_eff,
                           "covered": bench_eval.covered(task, final_names)})
    allowed = bench_eval.allowed_names(task)
    for c in calls:
        c["unnecessary"] = (not c["discovery"]) and c["capability"] is not None and c["capability"] not in allowed
        c["unknown"] = c["error_code"] == "unknown_capability"
    return {
        "final_answer": final_answer, "ended": ended, "infra_error": infra_error,
        "infra_failure": ended == "infra_failure",
        "rounds": rounds, "calls": calls, "expansions": [{k: v for k, v in e.items()} for e in expansion_events],
        "final_exposed_names": list(final_names), "coverage_trace": coverage_trace, "wall_ms": wall_ms,
        "selection": selection, "base_messages": base_messages, "_round_tools": round_tools,
        "trace_id": trace_id, "workspace_events": dict(authority.bench_events),
    }


def measure_run_tokens(cx: dict, run: dict) -> None:
    """Outside the timed window: per-round tool-schema cost by prompt difference (memoized by digest)."""
    counter = cx["counter"]
    base_messages = run["base_messages"]
    base = counter.count_prompt(base_messages, None)
    run["base_context"] = base
    per_round = []
    for tools in run["_round_tools"]:
        per_round.append(max(counter.count_prompt(base_messages, tools) - base, 0))
    run["tool_schema_tokens"] = per_round


def pair_measure(cx: dict, task: dict, runs: dict) -> dict:
    """interplane.crossaxis.measure on the round-1 sets; all-rounds and completion from usage."""
    full_caps = cx["catalog"].capabilities
    sel_names = runs["B"]["rounds"][0]["exposed_names"] if "B" in runs and runs["B"]["rounds"] else None
    if sel_names is None:
        return {}
    sel_caps = [cx["by_name"][n] for n in sel_names]

    def renderer(caps):
        tools = openai_tools_renderer(caps)
        return tools + [DISCOVERY_SPEC] if caps is sel_caps else tools

    allp = {"full": sum(r["prompt_tokens"] or 0 for r in runs["A"]["rounds"]),
            "selected": sum(r["prompt_tokens"] or 0 for r in runs["B"]["rounds"])} if "A" in runs else None
    comp = {"full": sum(r["completion_tokens"] or 0 for r in runs["A"]["rounds"]),
            "selected": sum(r["completion_tokens"] or 0 for r in runs["B"]["rounds"])} if "A" in runs else None
    return measure(full_caps, sel_caps, renderer, cx["counter"], runs["A"]["base_messages"] if "A" in runs else runs["B"]["base_messages"],
                   all_rounds_prompt=allp, completion=comp)


def make_receipt(cx: dict, task: dict, cond: str, run: dict, attempt: int, prior: list, pair_meas: dict, run_id: str) -> dict:
    tr = {
        "final_answer": run["final_answer"],
        "calls": [{k: c[k] for k in ("round", "tool", "capability", "decision", "status", "error_code", "discovery")} for c in run["calls"]],
        "expansions": [{"round": e["round"], "reason": e.get("reason", e.get("trigger")), "added": e.get("added", []), "refused": e.get("refused", [])} for e in run["expansions"]],
    }
    if run["infra_failure"]:
        verdict = {"success": False, "infra_failure": True, "checks": [], "failed": ["infra_failure"]}
    else:
        verdict = bench_eval.judge(task, cond, tr)
    metrics = bench_eval.run_metrics(task, cond, run)
    sel = run["selection"]
    selection_block = None
    if sel is not None:
        r1 = select(cx["catalog"], task["requested_domains"], always_include=ALWAYS_INCLUDE, renderer=openai_tools_renderer)[0]
        d = r1.to_dict()
        d["kind"] = "selection"
        selection_block = {"round1": d, "round1_digest": selection_digest(r1), "final_digest": selection_digest(sel),
                           "final_selected": [e["name"] for e in sel.selected]}
    return {
        "kind": "interplane_bench_receipt", "receipt_version": RECEIPT_VERSION, "run_id": run_id,
        "task": {"id": task["id"], "category": task["category"], "split": task["split"],
                 "digest": file_sha(cx["tasks_dir"] / f"{task['id']}.json")},
        "condition": cond, "attempt": attempt, "prior_infra_failures": prior,
        "system_prompt_digest": text_sha(cx["system_prompt"]),
        "user_request_digest": text_sha(task["user_request"]),
        "authority_profile": task["authority_profile"],
        "fixture": {"name": task["workspace_fixture"], "digest": cx["fixture_digests"][task["workspace_fixture"]]},
        "generation": cx["generation"],
        "discovery_tool": DISCOVERY if cond == "B" else None,
        "selection": selection_block,
        "rounds": run["rounds"], "calls": run["calls"], "expansions": run["expansions"],
        "final_exposed_names": run["final_exposed_names"] if cond == "B" else None,
        "coverage_trace": run["coverage_trace"],
        "ended": run["ended"], "infra_error": run["infra_error"],
        "transcript": tr, "judge": verdict, "metrics": metrics,
        "tool_schema_tokens_per_round": run.get("tool_schema_tokens"), "base_context_tokens": run.get("base_context"),
        "measure": pair_meas,
        "timings": {"wall_ms": run["wall_ms"], "model_ms": sum(r["latency_ms"] for r in run["rounds"])},
    }


def run_pair(cx: dict, task: dict, order: list, run_id: str, out: Path, conds: list) -> dict:
    prior: list = []
    runs: dict = {}
    for attempt in range(1, 4):
        runs = {}
        for cond in order:
            runs[cond] = run_condition(cx, task, cond, run_id)
        bad = [c for c in order if runs[c]["infra_failure"]]
        if not bad:
            break
        prior.append({"attempt": attempt, "conditions": bad, "errors": [runs[c]["infra_error"] for c in bad]})
    for cond in order:
        if not runs[cond]["infra_failure"]:
            measure_run_tokens(cx, runs[cond])
        else:
            runs[cond]["tool_schema_tokens"], runs[cond]["base_context"] = None, None
    pm = {}
    if "A" in runs and "B" in runs and not (runs["A"]["infra_failure"] or runs["B"]["infra_failure"]):
        pm = pair_measure(cx, task, runs)
    for cond in order:
        rc = make_receipt(cx, task, cond, runs[cond], len(prior) + 1, prior, pm, run_id)
        write_json(out / "receipts" / f"{task['id']}.{cond}.json", rc)
    return {c: runs[c] for c in order}


# ------------------------------------------------------------------------------- environment

def sh(cmd: str, timeout: float = 20) -> str:
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout).stdout.strip()
    except Exception as err:  # noqa: BLE001
        return f"unavailable: {type(err).__name__}"


def load_snapshot() -> dict:
    util = sh("nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits")
    procs = []
    for line in sh("pgrep -af 'llama-server|sglang.launch_server|vllm' ").splitlines():
        if "pgrep" in line or line.startswith(str(os.getpid())):
            continue
        m = re.search(r"--port (\d+)", line)
        kind = "llama-server" if "llama-server" in line else ("sglang" if "sglang" in line else "vllm")
        if "bash -c" in line:
            continue
        procs.append(f"{kind}:{m.group(1) if m else '?'}")
    return {"time": now(), "gpu_util_percent": int(util.split()[0]) if util[:1].isdigit() else None,
            "other_inference_processes": sorted(set(procs))}


def git_state(path: Path) -> dict:
    rev = sh(f"git -C {path} rev-parse HEAD")
    dirty = sh(f"git -C {path} status --porcelain -- . ':!bench/runs'") if path == REPO else ""
    return {"commit": rev, "dirty": bool(dirty)}


def environment(endpoint: str, model: str, odysseus_src: str) -> dict:
    ver = sh("curl -s " + endpoint.replace("/v1", "") + "/api/version")
    modelfile = sh(f"ollama show {model} --modelfile")
    m = re.search(r"sha256-([0-9a-f]{64})", modelfile)
    listing = [ln for ln in sh("ollama list").splitlines() if ln.startswith(model)]
    return {
        "ollama_version_cli": sh("ollama --version"),
        "ollama_api_version": ver,
        "model_blob_digest": ("sha256:" + m.group(1)) if m else None,
        "ollama_list_row": " ".join(listing[0].split()) if listing else None,
        "hardware": {"lscpu_head": sh("lscpu | head -12").splitlines(), "gpu": sh("nvidia-smi --query-gpu=name --format=csv,noheader"),
                     "free_g": sh("free -g").splitlines()},
        "os": {"uname": sh("uname -a"), "os_release": sh("cat /etc/os-release").splitlines()},
        "odysseus": {"src": odysseus_src, "commit": sh(f"git -C {odysseus_src} rev-parse HEAD")},
    }


# ------------------------------------------------------------------------------- selection and identity

def load_tasks(tasks_dir: Path, which: str, only: list) -> list:
    tasks = [json.loads(p.read_text(encoding="utf-8")) for p in sorted(tasks_dir.glob("*.json"))]
    if which != "all":
        tasks = [t for t in tasks if t["split"] == which]
    if only:
        tasks = [t for t in tasks if t["id"] in only]
    return sorted(tasks, key=lambda t: t["id"])


def build_identity(cx: dict, tasks: list) -> dict:
    catalog = cx["catalog"]
    per = {}
    sel_probe = None
    for t in tasks:
        sel, caps = select(catalog, t["requested_domains"], always_include=ALWAYS_INCLUDE, renderer=openai_tools_renderer)
        sel_probe = sel
        b_tools = openai_tools_renderer(caps) + [DISCOVERY_SPEC]
        a_tools = openai_tools_renderer(catalog.capabilities)
        per[t["id"]] = {
            "task_digest": file_sha(cx["tasks_dir"] / f"{t['id']}.json"),
            "requested_domains": t["requested_domains"],
            "fixture_digest": cx["fixture_digests"][t["workspace_fixture"]],
            "stub_digests": {k: file_sha(cx["bench"] / v["path"]) for k, v in sorted(t["stub_results"].items())},
            "fault_injection_digest": digest_of(t["fault_injection"]),
            "authority_profile": t["authority_profile"],
            "round1_selection_digest": selection_digest(sel),
            "round1_selected": [c.name for c in caps],
            "tools_digest": {"A": digest_of(a_tools), "B_round1": digest_of(b_tools)},
            "tools_bytes": {"A": len(jcs(a_tools).encode()), "B_round1": len(jcs(b_tools).encode())},
        }
    ident = {
        "catalog_digest": catalog.catalog_digest, "catalog_capabilities": len(catalog.capabilities),
        "selector": sel_probe.selector if sel_probe else None,
        "always_include": ALWAYS_INCLUDE, "discovery_tool": {"name": DISCOVERY, "digest": digest_of(DISCOVERY_SPEC)},
        "system_prompt_digest": text_sha(cx["system_prompt"]), "generation": cx["generation"],
        "per_task": per,
    }
    if cx["backends"] != "reference":  # the 0.2 reference identity stays byte-identical
        ident["backends"] = backends_identity(cx)
    ident["identity_digest"] = digest_of(ident)
    return ident


def backends_identity(cx: dict) -> dict:
    stores = sorted(sim_backends.STORES_DIR.glob("*.json"))
    return {"mode": cx["backends"], "registry_sha256": file_sha(sim_backends.BACKENDS_PATH),
            "sim_backends_sha256": file_sha(Path(sim_backends.__file__)),
            "stores": {p.name: file_sha(p) for p in stores}}


def make_cx(args, tasks_dir: Path) -> dict:
    catalog = catalog_with_domains()
    sp = (BENCH / "prompts" / "system.md").read_text(encoding="utf-8")
    fixtures = sorted({(t["workspace_fixture"]) for t in load_tasks(tasks_dir, "all", [])})
    client = Client(args.endpoint, args.model, args.timeout, args.seed, args.temperature, args.max_tokens)
    return {
        "bench": BENCH, "tasks_dir": tasks_dir, "catalog": catalog, "by_name": {c.name: c for c in catalog.capabilities},
        "system_prompt": sp, "client": client, "counter": LiveCounter(client), "max_rounds": args.max_rounds,
        "fixture_digests": {f: tree_digest(BENCH / f) for f in fixtures},
        "backends": args.backends,
        "backend_registry": sim_backends.load_backends() if args.backends == "sim-1" else None,
        "generation": {"temperature": args.temperature, "seed": args.seed, "max_tokens": args.max_tokens,
                       "timeout_s": args.timeout, "max_rounds": args.max_rounds, "dialect": "openai"},
    }


def check_corpus(strict: bool) -> dict:
    import validate  # bench/tools/validate.py

    frozen, now_d = validate.read_digest_file(), validate.digests()
    for k in ("tasks_digest", "inputs_digest", "protocol_sha256"):
        if frozen.get(k) != now_d.get(k):
            if strict:
                raise SystemExit(f"corpus digest {k} differs from CORPUS-DIGEST.txt: refusing to run")
    return {"frozen": {k: frozen.get(k) for k in ("tasks_digest", "inputs_digest", "protocol_sha256")}, "recomputed": now_d,
            "match": all(frozen.get(k) == now_d.get(k) for k in ("tasks_digest", "inputs_digest", "protocol_sha256"))}


def token_stability(cx: dict, task: dict) -> dict:
    """Same (messages, tools) counted twice must give the same number (no prompt-cache undercount)."""
    msgs = [{"role": "system", "content": cx["system_prompt"]}, {"role": "user", "content": task["user_request"]}]
    tools = openai_tools_renderer(cx["catalog"].capabilities)
    c = LiveCounter(cx["client"])
    a = c.count_prompt(msgs, tools)
    c.cache.clear()
    b = c.count_prompt(msgs, tools)
    c.cache.clear()
    base1 = c.count_prompt(msgs, None)
    return {"task": task["id"], "tools_count_1": a, "tools_count_2": b, "stable": a == b, "base_context": base1}


# ------------------------------------------------------------------------------- main

def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--tasks", choices=["dev", "qual", "all"], default="dev")
    ap.add_argument("--condition", choices=["A", "B", "both"], default="both")
    ap.add_argument("--endpoint", default="http://127.0.0.1:11434/v1")
    ap.add_argument("--model", default="qwen3.5:9b")
    ap.add_argument("--out", required=True)
    ap.add_argument("--probe", default=str(REPO / "qualification/reports/qwen35-9b-ollama-0.34.0.probe.json"))
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--max-rounds", type=int, default=8)
    ap.add_argument("--max-tokens", type=int, default=4096)
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--temperature", type=float, default=0.0)
    ap.add_argument("--only", default="", help="comma-separated task ids (debugging)")
    ap.add_argument("--resume", action="store_true")
    ap.add_argument("--prepare", action="store_true", help="write manifest.json (identity) and exit; no model request")
    ap.add_argument("--no-warmup", action="store_true")
    ap.add_argument("--tasks-dir", default=str(BENCH / "tasks"))
    ap.add_argument("--allow-nonfrozen", action="store_true", help="offline tests with synthetic tasks only")
    ap.add_argument("--backends", choices=["reference", "sim-1"], default="reference",
                    help="reference = 0.2 behaviour (only read_file/ls/glob/grep and task stubs execute); "
                         "sim-1 = bench/stubs/backends.json simulated backends")
    args = ap.parse_args(argv)

    out = Path(args.out)
    run_id = out.name.rstrip("/")
    tasks_dir = Path(args.tasks_dir)
    src = os.environ.get("ODYSSEUS_SRC", "")
    ody = _odysseus.load()
    if ody is None or not src:
        raise SystemExit("set ODYSSEUS_SRC to an Odysseus checkout at 2992bf6")
    if os.path.realpath(ody.root) != os.path.realpath(src) and not os.path.realpath(ody.root).startswith(os.path.realpath(src)):
        raise SystemExit(f"Odysseus loaded from {ody.root}, not ODYSSEUS_SRC={src}")
    ody_commit = sh(f"git -C {src} rev-parse HEAD")
    if not ody_commit.startswith(ODYSSEUS_COMMIT) and not args.allow_nonfrozen:
        raise SystemExit(f"Odysseus is at {ody_commit}, expected {ODYSSEUS_COMMIT}")
    cx = make_cx(args, tasks_dir)
    if text_sha(cx["system_prompt"]) != "sha256:" + FROZEN_PROMPT_SHA and not args.allow_nonfrozen:
        raise SystemExit("prompts/system.md does not match the digest every task pins")
    corpus = check_corpus(strict=not args.allow_nonfrozen)
    tasks = load_tasks(tasks_dir, args.tasks, [s for s in args.only.split(",") if s])
    if not tasks:
        raise SystemExit("no tasks selected")
    # The real authority must be able to decide: fail early, not closed-and-silent.
    probe_ws = tempfile.mkdtemp(prefix="interplane-bench-probe-")
    try:
        if not OdysseusAuthority(probe_ws, admin=True).available:
            raise SystemExit("OdysseusAuthority is not available")
    finally:
        shutil.rmtree(probe_ws, ignore_errors=True)

    identity = build_identity(cx, tasks)
    probe = json.loads(Path(args.probe).read_text(encoding="utf-8"))
    env = environment(args.endpoint, args.model, src)
    conds = ["A", "B"] if args.condition == "both" else [args.condition]
    manifest_path = out / "manifest.json"
    manifest = {
        "kind": "interplane_bench_manifest", "manifest_version": "0.2.0", "run_id": run_id,
        "tasks": args.tasks, "task_ids": [t["id"] for t in tasks], "conditions": conds,
        "interplane": git_state(REPO), "runner": {"run_bench_sha256": file_sha(Path(__file__)), "bench_eval_sha256": file_sha(BENCH / "tools" / "bench_eval.py")},
        "odysseus": env["odysseus"],
        "model": {"id": args.model, "digest": probe.get("model_digest"), "blob_digest_from_ollama": env["model_blob_digest"],
                  "ollama_list_row": env["ollama_list_row"]},
        "backend": {"name": probe.get("backend"), "version_probe": probe.get("backend_version"), "version_cli": env["ollama_version_cli"],
                    "version_api": env["ollama_api_version"], "endpoint": args.endpoint, "dialect": "openai"},
        "probe_report": {"path": os.path.relpath(args.probe, REPO) if str(args.probe).startswith(str(REPO)) else args.probe,
                         "digest": digest_of(probe), "file_sha256": file_sha(Path(args.probe))},
        "hardware": env["hardware"], "os": env["os"],
        "selector": identity["selector"],
        "expansion": {"mechanism": "interplane.crossaxis.expand (spec/CROSSAXIS.md Bounded expansion, v0.2)",
                      "max_expansions": 2, "max_added_per_expansion": 8, "discover_limit": 8,
                      "evidence": ["requested_excluded", "discovery_hit"]},
        "catalog": {"digest": identity["catalog_digest"], "capabilities": identity["catalog_capabilities"]},
        "corpus": corpus, "system_prompt_digest": identity["system_prompt_digest"], "generation": cx["generation"],
        "backends": identity.get("backends", {"mode": "reference"}),
        "order_rule": "tasks in id order; A first for even 0-based index, B first for odd (PROTOCOL-0.2.md section 2)",
        "deterministic_identity": identity,
    }
    if manifest_path.exists():
        old = json.loads(manifest_path.read_text(encoding="utf-8"))
        if old.get("deterministic_identity", {}).get("identity_digest") != identity["identity_digest"]:
            raise SystemExit("deterministic identity differs from the existing manifest.json: refusing to continue this run")
        manifest = {**old, "interplane": old.get("interplane", manifest["interplane"]), "runner": old.get("runner", manifest["runner"])}
        if manifest["runner"]["run_bench_sha256"] != file_sha(Path(__file__)):
            raise SystemExit("run_bench.py changed since the manifest was written")
    if args.prepare:
        manifest["stochastic_execution"] = {"prepared_at": now(), "note": "manifest written before the first model request"}
        manifest["concurrent_load"] = {"at_prepare": load_snapshot()}
        write_json(manifest_path, manifest)
        print(f"manifest written: {manifest_path} identity {identity['identity_digest']}")
        return 0

    stoch = manifest.get("stochastic_execution", {})
    stoch["started_at"] = stoch.get("started_at") or now()
    samples = [load_snapshot()]
    stoch["token_stability"] = token_stability(cx, tasks[0])
    if not args.no_warmup and not manifest.get("stochastic_execution", {}).get("warmup"):
        warm = next((t for t in load_tasks(tasks_dir, "dev", ["filesystem-001"])), None)
        if warm is not None:
            t0 = time.monotonic()
            try:
                run_condition(cx, warm, "B", run_id + "-warmup")
                stoch["warmup"] = {"task": warm["id"], "condition": "B", "wall_ms": int((time.monotonic() - t0) * 1000), "timed": False}
            except InfraError as err:
                stoch["warmup"] = {"task": warm["id"], "error": str(err)}
    manifest["stochastic_execution"] = stoch
    manifest["concurrent_load"] = {"at_start": samples[0]}
    write_json(manifest_path, manifest)

    print(f"run {run_id}: {len(tasks)} tasks, conditions {conds}, identity {identity['identity_digest']}", flush=True)
    done = 0
    for idx, task in enumerate(tasks):
        rpaths = [out / "receipts" / f"{task['id']}.{c}.json" for c in conds]
        if args.resume and all(p.exists() for p in rpaths):
            continue
        order = list(conds) if len(conds) == 1 else (["A", "B"] if idx % 2 == 0 else ["B", "A"])
        samples.append(load_snapshot())
        res = run_pair(cx, task, order, run_id, out, conds)
        done += 1
        line = " ".join(
            f"{c}:{'ok' if json.loads(p.read_text())['judge']['success'] else 'FAIL'}/{res[c]['wall_ms']//1000}s/{len(res[c]['rounds'])}r"
            for c, p in zip(order, [out / 'receipts' / f"{task['id']}.{c}.json" for c in order]))
        print(f"[{idx+1}/{len(tasks)}] {task['id']} {line}", flush=True)
    samples.append(load_snapshot())
    stoch["finished_at"] = now()
    stoch["counter_requests_in_last_process"] = cx["counter"].requests
    manifest["stochastic_execution"] = stoch
    utils = [s["gpu_util_percent"] for s in samples if s["gpu_util_percent"] is not None]
    others = sorted({p for s in samples for p in s["other_inference_processes"]})
    old_load = manifest.get("concurrent_load", {})
    old_samples = old_load.get("samples", [])
    manifest["concurrent_load"] = {
        "at_start": old_load.get("at_start", samples[0]), "at_end": samples[-1],
        "samples": old_samples + samples[1:-1],
        "other_inference_processes_seen": sorted(set(others) | set(old_load.get("other_inference_processes_seen", []))),
        "gpu_util_percent_max_sampled_at_pair_starts": max(utils + [old_load.get("gpu_util_percent_max_sampled_at_pair_starts") or 0]) if utils else None,
        "note": "sampled at start, before each pair, and at end; wall-clock is a reported metric and is affected by other GPU users",
    }
    write_json(manifest_path, manifest)
    print(f"done: {done} pairs run; manifest {manifest_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
