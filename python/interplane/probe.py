"""Interplane Probe (Python reference): qualify an OpenAI-compatible endpoint with real requests.

Every verdict cites a request digest and a response digest. Nothing is inferred from the model
name. The probe speaks plain HTTP (stdlib ``urllib``); no serving engine is a dependency.
Credentials come from ``INTERPLANE_PROBE_API_KEY`` and are never written to the report.
"""

import argparse
import json
import os
import platform
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from typing import Optional

from .core import ProbeReport, jcs, sha256_hex
from .lenshift import qwen35 as _qwen35

PROBE_VERSION = "0.1.0"
PROTOCOL = "0.1"

PROFILES = {
    "interplane.core.0.1": ["chat.basic", "models.list"],
    "lenshift.openai.1": ["tools.native", "tools.result_replay"],
    "lenshift.qwen35.1": ["tools.text_qwen35", "tools.result_replay"],
    "relayline.tool_replay.1": ["tools.result_replay", "chat.streaming"],
}

_READ_FILE_TOOL = {
    "type": "function",
    "function": {
        "name": "read_file",
        "description": "Read a UTF-8 text file from the workspace.",
        "parameters": {
            "type": "object",
            "properties": {"path": {"type": "string", "description": "Relative path."}},
            "required": ["path"],
        },
    },
}
_LIST_DIR_TOOL = {
    "type": "function",
    "function": {
        "name": "list_dir",
        "description": "List the entries of a workspace directory.",
        "parameters": {
            "type": "object",
            "properties": {"path": {"type": "string", "description": "Relative path."}},
            "required": ["path"],
        },
    },
}

_QWEN35_TOOL_INSTRUCTION = (
    "# Tools\n\nYou have access to the following functions:\n\n<tools>\n"
    + json.dumps(_READ_FILE_TOOL["function"], ensure_ascii=False)
    + "\n</tools>\n\n"
    "If you choose to call a function ONLY reply in the following format with NO suffix:\n\n"
    "<tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\n"
    "value_1\n</parameter>\n</function>\n</tool_call>\n\n"
    "<IMPORTANT>\nReminder:\n- Function calls MUST follow the specified format\n"
    "- Required parameters MUST be specified\n</IMPORTANT>"
)


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class Endpoint:
    def __init__(self, base_url: str, model: str, timeout: float = 120.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.timeout = timeout
        self.api_key = os.environ.get("INTERPLANE_PROBE_API_KEY")

    def _headers(self) -> dict:
        h = {"Content-Type": "application/json", "Accept": "application/json"}
        if self.api_key:
            h["Authorization"] = f"Bearer {self.api_key}"
        return h

    def post(self, path: str, body: dict) -> tuple:
        """Returns (status, raw_text, request_digest, response_digest)."""
        data = jcs(body).encode("utf-8")
        req = urllib.request.Request(
            self.base_url + path, data=data, headers=self._headers(), method="POST"
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read().decode("utf-8", "replace")
                status = resp.status
        except urllib.error.HTTPError as err:
            raw = err.read().decode("utf-8", "replace")
            status = err.code
        return status, raw, "sha256:" + sha256_hex(data), "sha256:" + sha256_hex(raw)

    def get(self, path: str) -> tuple:
        req = urllib.request.Request(self.base_url + path, headers=self._headers(), method="GET")
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read().decode("utf-8", "replace")
                status = resp.status
        except urllib.error.HTTPError as err:
            raw = err.read().decode("utf-8", "replace")
            status = err.code
        return status, raw, "sha256:" + sha256_hex(("GET " + path).encode()), "sha256:" + sha256_hex(raw)

    def chat(self, body: dict) -> tuple:
        body = dict(body)
        body.setdefault("model", self.model)
        body.setdefault("temperature", 0)
        body.setdefault("seed", 42)
        return self.post("/chat/completions", body)


def _message(raw: str) -> Optional[dict]:
    try:
        obj = json.loads(raw)
        return obj["choices"][0]["message"]
    except (ValueError, KeyError, IndexError, TypeError):
        return None


def _content(msg: Optional[dict]) -> str:
    if not msg:
        return ""
    c = msg.get("content")
    return c if isinstance(c, str) else ""


def _tool_calls(msg: Optional[dict]) -> list:
    if not msg or not isinstance(msg.get("tool_calls"), list):
        return []
    return msg["tool_calls"]


def _args_object(call: dict) -> Optional[dict]:
    fn = call.get("function") if isinstance(call, dict) else None
    if not isinstance(fn, dict):
        return None
    args = fn.get("arguments")
    if isinstance(args, dict):
        return args
    if isinstance(args, str):
        try:
            parsed = json.loads(args)
        except ValueError:
            return None
        return parsed if isinstance(parsed, dict) else None
    return None


def _sse_content(raw: str) -> tuple:
    """Assemble SSE text; returns (content, chunk_count)."""
    content, chunks = [], 0
    for line in raw.splitlines():
        if not line.startswith("data:"):
            continue
        body = line[5:].strip()
        if body == "[DONE]":
            continue
        chunks += 1
        try:
            obj = json.loads(body)
            delta = obj["choices"][0].get("delta", {})
        except (ValueError, KeyError, IndexError, TypeError):
            continue
        if isinstance(delta.get("content"), str):
            content.append(delta["content"])
    return "".join(content), chunks


class Prober:
    def __init__(self, ep: Endpoint) -> None:
        self.ep = ep
        self.records: list = []
        self._native_call: Optional[dict] = None

    # -- helpers -----------------------------------------------------------

    def _run(self, name: str, fn) -> dict:
        t0 = time.monotonic()
        verdict, detail, req_d, resp_d = "UNKNOWN", None, None, None
        try:
            verdict, detail, req_d, resp_d = fn()
        except urllib.error.URLError as err:
            verdict, detail = "FAIL", f"transport error: {getattr(err, 'reason', err)}"
        except (OSError, ValueError) as err:
            verdict, detail = "FAIL", f"error: {err}"
        rec = {
            "name": name,
            "verdict": verdict,
            "detail": detail,
            "duration_ms": int((time.monotonic() - t0) * 1000),
            "attempts": 1,
            "request_digest": req_d,
            "response_digest": resp_d,
        }
        self.records.append(rec)
        return rec

    # -- probes ------------------------------------------------------------

    def models_list(self):
        status, raw, rq, rs = self.ep.get("/models")
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        try:
            ids = [m.get("id") for m in json.loads(raw).get("data", [])]
        except (ValueError, AttributeError):
            return "FAIL", "response is not a model list", rq, rs
        if self.ep.model in ids:
            return "PASS", f"model listed among {len(ids)} entries", rq, rs
        return "DEGRADED", f"endpoint listed {len(ids)} models but not '{self.ep.model}'", rq, rs

    def chat_basic(self):
        status, raw, rq, rs = self.ep.chat(
            {"messages": [{"role": "user", "content": "Reply with the single word: ready"}], "max_tokens": 2048}
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        msg = _message(raw)
        if _content(msg).strip():
            return "PASS", None, rq, rs
        if isinstance(msg, dict) and isinstance(msg.get("reasoning"), str):
            return "FAIL", "empty content; reasoning channel consumed the completion budget", rq, rs
        return "FAIL", "empty content", rq, rs

    def chat_streaming(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [{"role": "user", "content": "Reply with the single word: ready"}],
                "stream": True,
                "max_tokens": 2048,
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        content, chunks = _sse_content(raw)
        if chunks == 0:
            return "UNSUPPORTED", "no SSE chunks", rq, rs
        return ("PASS" if content.strip() else "FAIL"), f"{chunks} chunks", rq, rs

    def tools_native(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [{"role": "user", "content": "Read the file README.md and tell me what it says."}],
                "tools": [_READ_FILE_TOOL],
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        msg = _message(raw)
        calls = _tool_calls(msg)
        if not calls:
            return "UNSUPPORTED", "no tool_calls in response", rq, rs
        call = calls[0]
        name = (call.get("function") or {}).get("name") if isinstance(call, dict) else None
        args = _args_object(call)
        if name == "read_file" and args is not None and "path" in args:
            self._native_call = call
            return "PASS", "tool_calls[0] = read_file with object arguments", rq, rs
        return "DEGRADED", f"tool call present but name={name!r} args_ok={args is not None}", rq, rs

    def tools_text_qwen35(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [
                    {"role": "system", "content": _QWEN35_TOOL_INSTRUCTION},
                    {"role": "user", "content": "Read the file README.md and tell me what it says."},
                ],
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        msg = _message(raw)
        text = _content(msg)
        if _tool_calls(msg):
            return (
                "DEGRADED",
                "backend parsed the text form into native tool_calls itself (content empty)",
                rq,
                rs,
            )
        parsed = _qwen35.parse(text, self.ep.model, "probe", 1)
        if parsed.intents and parsed.intents[0].tool.name == "read_file":
            return "PASS", "content holds a parseable <tool_call><function=read_file> block", rq, rs
        if "<tool_call>" in text:
            return "DEGRADED", "tool_call markup present but not parseable as qwen35", rq, rs
        if not text.strip():
            return (
                "UNSUPPORTED",
                "empty content and no tool_calls: the backend parser consumed or suppressed the "
                "text-form call (compare a raw-completion capture)",
                rq,
                rs,
            )
        return "UNSUPPORTED", "no <tool_call> markup in content", rq, rs

    def tools_parallel(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [
                    {
                        "role": "user",
                        "content": "Read both README.md and LICENSE now, in one go, then summarize both.",
                    }
                ],
                "tools": [_READ_FILE_TOOL],
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        calls = _tool_calls(_message(raw))
        if len(calls) >= 2:
            return "PASS", f"{len(calls)} tool_calls in one turn", rq, rs
        if len(calls) == 1:
            return "DEGRADED", "only one tool call for a two-call task", rq, rs
        return "UNSUPPORTED", "no tool_calls", rq, rs

    def tools_result_replay(self):
        call = self._native_call or {
            "id": "call_probe0001",
            "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"README.md\"}"},
        }
        call_id = call.get("id") or "call_probe0001"
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [
                    {"role": "user", "content": "Read the file README.md and tell me what it says."},
                    {"role": "assistant", "content": "", "tool_calls": [call]},
                    {
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": "{\"content\": \"INTERPLANE probe marker ZEBRA-7731.\"}",
                    },
                ],
                "tools": [_READ_FILE_TOOL],
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        msg = _message(raw)
        text = _content(msg)
        if "ZEBRA-7731" in text:
            return "PASS", "final answer quotes the injected result", rq, rs
        if _tool_calls(msg):
            return "DEGRADED", "model re-issued a tool call instead of answering", rq, rs
        if text.strip():
            return "DEGRADED", "answered without using the injected result", rq, rs
        return "FAIL", "empty answer", rq, rs

    def tools_unknown_refusal(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [{"role": "user", "content": "Use the compile_project tool to build the project."}],
                "tools": [_READ_FILE_TOOL, _LIST_DIR_TOOL],
            }
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        calls = _tool_calls(_message(raw))
        names = [(c.get("function") or {}).get("name") for c in calls if isinstance(c, dict)]
        if not names:
            return "PASS", "no tool call for the nonexistent tool", rq, rs
        if "compile_project" in names:
            return "FAIL", "model called a tool that was not offered", rq, rs
        return "DEGRADED", f"model called an offered tool instead: {names}", rq, rs

    def reasoning_channel(self):
        status, raw, rq, rs = self.ep.chat(
            {"messages": [{"role": "user", "content": "What is 17 times 23? Answer with the number."}]}
        )
        if status != 200:
            return "FAIL", f"HTTP {status}", rq, rs
        msg = _message(raw) or {}
        for key in ("reasoning_content", "reasoning"):
            if isinstance(msg.get(key), str) and msg[key].strip():
                return "PASS", f"reasoning present as message.{key}", rq, rs
        if "<think>" in _content(msg):
            return "PASS", "reasoning present inline as <think>", rq, rs
        return "UNSUPPORTED", "no reasoning channel observed", rq, rs

    def reasoning_disable(self):
        mechanisms = [
            ("chat_template_kwargs.enable_thinking=false", {"chat_template_kwargs": {"enable_thinking": False}}),
            ("think=false", {"think": False}),
            ("reasoning_effort=none", {"reasoning_effort": "none"}),
        ]
        last = (None, None)
        for label, extra in mechanisms:
            body = {"messages": [{"role": "user", "content": "What is 17 times 23? Answer with the number."}]}
            body.update(extra)
            status, raw, rq, rs = self.ep.chat(body)
            last = (rq, rs)
            if status != 200:
                continue
            msg = _message(raw) or {}
            has = any(isinstance(msg.get(k), str) and msg[k].strip() for k in ("reasoning_content", "reasoning"))
            if not has and "<think>" not in _content(msg):
                return "PASS", f"reasoning absent with {label}", rq, rs
        return "UNSUPPORTED", "reasoning present under every known disable mechanism", last[0], last[1]

    def json_structured(self):
        status, raw, rq, rs = self.ep.chat(
            {
                "messages": [{"role": "user", "content": "Return a JSON object with key answer set to 42."}],
                "response_format": {"type": "json_object"},
            }
        )
        if status != 200:
            return "UNSUPPORTED", f"HTTP {status}", rq, rs
        text = _content(_message(raw)).strip()
        try:
            obj = json.loads(text)
        except ValueError:
            return "FAIL", "content is not valid JSON", rq, rs
        return ("PASS" if isinstance(obj, dict) else "DEGRADED"), None, rq, rs

    def context_probe(self, k: int):
        def fn():
            filler = ("The quick brown fox jumps over the lazy dog. " * 8 + "\n")
            needle = "The secret code word is PELICAN-4410."
            target_bytes = k * 1024 * 3  # ~3 bytes per token, rough
            lines = []
            size = 0
            while size < target_bytes:
                lines.append(filler)
                size += len(filler)
            mid = len(lines) // 2
            lines.insert(mid, needle + "\n")
            prompt = "".join(lines) + "\n\nWhat is the secret code word? Answer with just the code word."
            status, raw, rq, rs = self.ep.chat({"messages": [{"role": "user", "content": prompt}], "max_tokens": 2048})
            if status != 200:
                return "FAIL", f"HTTP {status}", rq, rs
            text = _content(_message(raw))
            if "PELICAN-4410" in text:
                return "PASS", None, rq, rs
            return "DEGRADED", "answered but did not return the needle", rq, rs

        return fn

    # -- driver ------------------------------------------------------------

    def run_all(self, skip: Optional[set] = None) -> list:
        skip = skip or set()
        order = [
            ("models.list", self.models_list),
            ("chat.basic", self.chat_basic),
            ("chat.streaming", self.chat_streaming),
            ("tools.native", self.tools_native),
            ("tools.text_qwen35", self.tools_text_qwen35),
            ("tools.parallel", self.tools_parallel),
            ("tools.result_replay", self.tools_result_replay),
            ("tools.unknown_refusal", self.tools_unknown_refusal),
            ("reasoning.channel", self.reasoning_channel),
            ("reasoning.disable", self.reasoning_disable),
            ("json.structured", self.json_structured),
            ("context.8k", self.context_probe(8)),
            ("context.16k", self.context_probe(16)),
            ("context.32k", self.context_probe(32)),
        ]
        for name, fn in order:
            if name in skip:
                self.records.append(
                    {"name": name, "verdict": "SKIPPED", "detail": "skipped by operator", "duration_ms": None,
                     "attempts": None, "request_digest": None, "response_digest": None}
                )
            else:
                self._run(name, fn)
        return self.records


def profiles_for(records: list) -> list:
    verdicts = {r["name"]: r["verdict"] for r in records}
    out = []
    for name, required in PROFILES.items():
        vs = [verdicts.get(p) for p in required]
        if all(v == "PASS" for v in vs):
            status = "compatible"
        elif any(v in (None, "SKIPPED", "UNKNOWN") for v in vs):
            status = "untested"
        else:
            status = "incompatible"
        out.append({"name": name, "status": status, "required_probes": list(required)})
    return out


def environment(ep: Endpoint, backend: Optional[str], backend_version: Optional[str]) -> dict:
    return {
        "hardware": platform.machine(),
        "os": f"{platform.system()} {platform.release()}",
        "endpoint_type": "openai_compatible_http",
        "interplane_version": PROTOCOL,
        "probe_version": PROBE_VERSION,
        "backend": backend,
        "backend_version": backend_version,
        "python": platform.python_version(),
    }


def probe(
    base_url: str,
    model: str,
    *,
    backend: Optional[str] = None,
    backend_version: Optional[str] = None,
    model_digest: Optional[str] = None,
    skip: Optional[set] = None,
    timeout: float = 120.0,
) -> ProbeReport:
    ep = Endpoint(base_url, model, timeout=timeout)
    started = now()
    records = Prober(ep).run_all(skip)
    report = ProbeReport(
        probe_version=PROBE_VERSION,
        endpoint=ep.base_url,
        model=model,
        started_at=started,
        probes=records,
        profiles=profiles_for(records),
        backend=backend,
        backend_version=backend_version,
        model_digest=model_digest,
        finished_at=now(),
        extensions={"environment": environment(ep, backend, backend_version)},
    )
    return report


def report_dict(report: ProbeReport) -> dict:
    d = report.to_dict()
    d["kind"] = ProbeReport.KIND
    env = (d.get("extensions") or {}).pop("environment", None)
    if env is not None:
        d["environment"] = env
    if not d.get("extensions"):
        d.pop("extensions", None)
    return d


def main(argv: Optional[list] = None) -> int:
    ap = argparse.ArgumentParser(prog="interplane-probe")
    ap.add_argument("base_url", help="OpenAI-compatible base URL, e.g. http://127.0.0.1:11434/v1")
    ap.add_argument("--model", required=True)
    ap.add_argument("--backend")
    ap.add_argument("--backend-version")
    ap.add_argument("--model-digest")
    ap.add_argument("--skip", action="append", default=[], help="probe name to skip (repeatable)")
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--out", help="write the probe_report JSON here")
    args = ap.parse_args(argv)
    rep = probe(
        args.base_url,
        args.model,
        backend=args.backend,
        backend_version=args.backend_version,
        model_digest=args.model_digest,
        skip=set(args.skip),
        timeout=args.timeout,
    )
    d = report_dict(rep)
    text = json.dumps(d, indent=2, sort_keys=True) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    for p in d["probes"]:
        print(f"{p['name']:<24} {p['verdict']:<12} {p.get('detail') or ''}")
    for pr in d["profiles"]:
        print(f"profile {pr['name']:<26} {pr['status']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
