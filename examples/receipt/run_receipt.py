"""Before/after receipt: one real agent task, full tool surface vs CrossAxis-selected surface.

Runs the same Odysseus-style task twice against a real OpenAI-compatible endpoint (Qwen3.5 on
Ollama by default):

  before: all 71 Odysseus tools rendered into the request
  after:  only the CrossAxis `domain_match` selection for the task's domains

Every model turn goes through the INTERPLANE pipeline (Lenshift openai -> Core admit -> CrossAxis
map -> Crossveil decide/execute) with `OdysseusAuthority` as the runtime: Odysseus's own gates
decide, Odysseus's own handlers execute the read-only tools inside a temporary workspace. The
backend's probe report is pinned into the receipt. Nothing here grants authority; the model only
expresses intent.

Run (needs an Odysseus checkout and its venv, see adapters/odysseus/README.md):

  ODYSSEUS_SRC=~/workspace/odysseus PYTHONPATH=python:adapters/odysseus \\
      <odysseus-venv>/bin/python examples/receipt/run_receipt.py \\
      --endpoint http://127.0.0.1:11434/v1 --model qwen3.5:9b \\
      --probe qualification/reports/qwen35-9b-ollama-0.34.0.probe.json \\
      --out examples/receipt/receipts/<name>.json

  --verify <receipt.json>  reruns and reports which sections reproduced byte-for-byte.

Tool arguments are never written to the receipt; only their digests and key names.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import sys
import tempfile
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from interplane.core import jcs, sha256_hex
from interplane.crossaxis import openai_tools_renderer, select
from interplane.crossveil import Pipeline
from interplane.lenshift import openai as _openai
from interplane_adapter_odysseus.authority import OdysseusAuthority
from interplane_adapter_odysseus.dialect import make_registry
from interplane_adapter_odysseus.domains import catalog_with_domains
from interplane_adapter_odysseus.mapping import mapping_table

RECEIPT_VERSION = "0.1.0"
MAX_ROUNDS = 6

WORKSPACE_FILES = {
    "README.md": "# Lanternfish\n\nLanternfish is a tiny tide-table generator.\n\nSee notes/todo.txt.\n",
    "notes/todo.txt": "- add moon phase\n- document the CLI\n",
    "src/main.rs": "fn main() { println!(\"lanternfish\"); }\n",
}
TASK_PROMPT = (
    "You are working inside a project workspace. Find out the project's name by reading its README, "
    "then answer with exactly one line: 'Project name: <name>'. Use tools; do not guess."
)
TASK_DOMAINS = ["filesystem"]
ALWAYS_INCLUDE = ["ask_user"]
EXPECTED_ANSWER_SUBSTRING = "Lanternfish"


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def digest_of(value) -> str:
    return "sha256:" + sha256_hex(jcs(value).encode("utf-8"))


def chat(endpoint: str, body: dict, timeout: float) -> tuple:
    data = jcs(body).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    key = os.environ.get("INTERPLANE_PROBE_API_KEY")
    if key:
        headers["Authorization"] = f"Bearer {key}"
    req = urllib.request.Request(endpoint.rstrip("/") + "/chat/completions", data=data, headers=headers)
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode("utf-8", "replace")
    return json.loads(raw), int((time.monotonic() - t0) * 1000), "sha256:" + sha256_hex(data)


def make_workspace(root: str) -> dict:
    digests = {}
    for rel, text in WORKSPACE_FILES.items():
        p = Path(root, rel)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
        digests[rel] = "sha256:" + sha256_hex(text.encode("utf-8"))
    return digests


def run_task(label: str, endpoint: str, model: str, tools: list, workspace: str, timeout: float) -> dict:
    """One agent loop through the INTERPLANE pipeline. Returns the receipt section."""
    authority = OdysseusAuthority(workspace, admin=True)
    if not authority.available:
        raise SystemExit("Odysseus is not importable: set ODYSSEUS_SRC")
    pipe = Pipeline(make_registry(), mapping_table(), authority)
    trace_id = f"receipt-{label}"
    messages = [{"role": "user", "content": TASK_PROMPT}]
    rounds = []
    final_answer = None
    for turn in range(MAX_ROUNDS):
        body = {"model": model, "messages": messages, "tools": tools, "temperature": 0, "seed": 42}
        resp, ms, req_digest = chat(endpoint, body, timeout)
        usage = resp.get("usage") or {}
        msg = resp["choices"][0]["message"]
        out = pipe.run_turn("openai", model, msg, trace_id, turn)
        calls = []
        for intent, rec, res in zip(out_intents(msg, model, trace_id, turn), out.observed, out.results):
            calls.append(
                {
                    "request_id": rec.request_id,
                    "tool": intent["tool"],
                    "argument_keys": intent["argument_keys"],
                    "arguments_digest": intent["arguments_digest"],
                    "observed": rec.to_dict(),
                    "result_status": res.status,
                    "result_trust": (res.provenance or {}).get("trust"),
                    "result_content_kind": (res.provenance or {}).get("content_kind"),
                }
            )
        rounds.append(
            {
                "turn": turn,
                "request_digest": req_digest,
                "prompt_tokens": usage.get("prompt_tokens"),
                "completion_tokens": usage.get("completion_tokens"),
                "latency_ms": ms,
                "outcome": out.outcome,
                "tool_calls": calls,
                "text_digest": ("sha256:" + sha256_hex(out.text.encode())) if out.text else None,
            }
        )
        if out.outcome != "tool_request":
            final_answer = out.text
            break
        assistant = {"role": "assistant", "content": msg.get("content") or "", "tool_calls": msg.get("tool_calls")}
        messages.append(assistant)
        messages.extend(out.rendered)
    success = bool(final_answer) and EXPECTED_ANSWER_SUBSTRING in final_answer
    return {
        "label": label,
        "tools_count": len(tools),
        "tools_rendered_bytes": len(jcs(tools).encode("utf-8")),
        "tools_digest": digest_of(tools),
        "rounds": rounds,
        "first_prompt_tokens": rounds[0]["prompt_tokens"] if rounds else None,
        "total_prompt_tokens": sum(r["prompt_tokens"] or 0 for r in rounds),
        "total_completion_tokens": sum(r["completion_tokens"] or 0 for r in rounds),
        "tool_calls_total": sum(len(r["tool_calls"]) for r in rounds),
        "executed_total": sum(1 for r in rounds for c in r["tool_calls"] if c["observed"]["execute_invoked"]),
        "final_answer": final_answer,
        "task_success": success,
        "runtime_decide_calls": sum(1 for r in rounds for c in r["tool_calls"] if c["observed"]["decide_invoked"]),
    }


def out_intents(msg: dict, model: str, trace_id: str, turn: int) -> list:
    """Tool name, argument key names and argument digest per parsed intent (arguments never kept).

    Re-parses the assistant message with the same Lenshift dialect the pipeline used; the parser
    is deterministic, so this is the pipeline's own view of the intents.
    """
    parsed = _openai.parse(msg, model, trace_id, turn)
    return [
        {
            "tool": intent.tool.name,
            "argument_keys": sorted(intent.arguments.keys()),
            "arguments_digest": digest_of(intent.arguments),
        }
        for intent in parsed.intents
    ]


def build(endpoint: str, model: str, probe_path: str, timeout: float) -> dict:
    catalog = catalog_with_domains()
    all_tools = openai_tools_renderer(catalog.capabilities)
    selection, selected_caps = select(
        catalog, TASK_DOMAINS, always_include=ALWAYS_INCLUDE, renderer=openai_tools_renderer
    )
    selected_tools = openai_tools_renderer(selected_caps)
    sel = selection.to_dict()
    sel["kind"] = "selection"

    probe = json.loads(Path(probe_path).read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="interplane-receipt-") as ws:
        files = make_workspace(ws)
        before = run_task("before", endpoint, model, all_tools, ws, timeout)
    with tempfile.TemporaryDirectory(prefix="interplane-receipt-") as ws:
        make_workspace(ws)
        after = run_task("after", endpoint, model, selected_tools, ws, timeout)

    b, a = before["first_prompt_tokens"], after["first_prompt_tokens"]
    receipt = {
        "kind": "interplane_receipt",
        "receipt_version": RECEIPT_VERSION,
        "created_at": now(),
        "environment": {
            "endpoint": endpoint,
            "model": model,
            "backend": probe.get("backend"),
            "backend_version": probe.get("backend_version"),
            "model_digest": probe.get("model_digest"),
            "host": f"{platform.system()} {platform.release()} {platform.machine()}",
            "python": platform.python_version(),
            "interplane_version": "0.1",
            "runtime_adapter": "interplane_adapter_odysseus (odysseus@2992bf6)",
        },
        "probe_report": {
            "path": probe_path,
            "digest": digest_of(probe),
            "profiles": {p["name"]: p["status"] for p in probe.get("profiles", [])},
            "verdicts": {p["name"]: p["verdict"] for p in probe.get("probes", [])},
        },
        "task": {
            "prompt": TASK_PROMPT,
            "domains": TASK_DOMAINS,
            "always_include": ALWAYS_INCLUDE,
            "workspace_files": files,
            "expected_answer_substring": EXPECTED_ANSWER_SUBSTRING,
            "max_rounds": MAX_ROUNDS,
        },
        "catalog": {"runtime": catalog.runtime, "catalog_digest": catalog.catalog_digest, "capabilities": len(catalog.capabilities)},
        "selection": sel,
        "before": before,
        "after": after,
        "delta": {
            "tools": {"before": before["tools_count"], "after": after["tools_count"]},
            "tools_rendered_bytes": {"before": before["tools_rendered_bytes"], "after": after["tools_rendered_bytes"]},
            "first_prompt_tokens": {"before": b, "after": a, "saved": (b - a) if (b is not None and a is not None) else None,
                                     "saved_pct": round(100.0 * (b - a) / b, 1) if b else None},
            "total_prompt_tokens": {"before": before["total_prompt_tokens"], "after": after["total_prompt_tokens"]},
            "task_success": {"before": before["task_success"], "after": after["task_success"]},
        },
    }
    receipt["deterministic_digest"] = deterministic_digest(receipt)
    return receipt


def deterministic_digest(receipt: dict) -> str:
    """Digest of the parts that must reproduce byte-for-byte regardless of model sampling."""
    det = {
        "catalog": receipt["catalog"],
        "selection": receipt["selection"],
        "task": receipt["task"],
        "tools_digest": {"before": receipt["before"]["tools_digest"], "after": receipt["after"]["tools_digest"]},
        "probe_digest": receipt["probe_report"]["digest"],
    }
    return digest_of(det)


def summarize(r: dict) -> str:
    d = r["delta"]
    lines = [
        f"model {r['environment']['model']} on {r['environment']['backend']} {r['environment']['backend_version']}",
        "probe profiles: " + ", ".join(f"{k}={v}" for k, v in r["probe_report"]["profiles"].items()),
        f"tools exposed: before {d['tools']['before']}  after {d['tools']['after']}",
        f"first prompt tokens: before {d['first_prompt_tokens']['before']}  after {d['first_prompt_tokens']['after']}  saved {d['first_prompt_tokens']['saved']} ({d['first_prompt_tokens']['saved_pct']}%)",
        f"task success: before {d['task_success']['before']}  after {d['task_success']['after']}",
        f"executed through Crossveil: before {r['before']['executed_total']}  after {r['after']['executed_total']}",
        f"deterministic digest: {r['deterministic_digest']}",
    ]
    return "\n".join(lines)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default="http://127.0.0.1:11434/v1")
    ap.add_argument("--model", default="qwen3.5:9b")
    ap.add_argument("--probe", default="qualification/reports/qwen35-9b-ollama-0.34.0.probe.json")
    ap.add_argument("--out")
    ap.add_argument("--verify", help="existing receipt to reproduce")
    ap.add_argument("--timeout", type=float, default=300.0)
    args = ap.parse_args(argv)
    receipt = build(args.endpoint, args.model, args.probe, args.timeout)
    print(summarize(receipt))
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"receipt written: {args.out}")
    if args.verify:
        old = json.loads(Path(args.verify).read_text(encoding="utf-8"))
        same_det = old["deterministic_digest"] == receipt["deterministic_digest"]
        same_before = old["before"]["final_answer"] == receipt["before"]["final_answer"]
        same_after = old["after"]["final_answer"] == receipt["after"]["final_answer"]
        same_tokens = old["delta"]["first_prompt_tokens"] == receipt["delta"]["first_prompt_tokens"]
        print(f"verify: deterministic sections {'REPRODUCED' if same_det else 'DIFFER'}; "
              f"model answers before={'same' if same_before else 'differ'} after={'same' if same_after else 'differ'}; "
              f"first prompt tokens {'same' if same_tokens else 'differ'}")
        return 0 if same_det else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
