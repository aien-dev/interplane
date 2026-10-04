"""Run model turns through the real INTERPLANE Pipeline against OdysseusAuthority.

    ODYSSEUS_SRC=/path/to/odysseus PYTHONPATH=<interplane>/python:<adapter dir> \
        <odysseus-venv>/bin/python -m interplane_adapter_odysseus.demo
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

from interplane.crossveil import Pipeline

from .authority import OdysseusAuthority
from .dialect import make_registry
from .mapping import mapping_table


def make_pipeline(authority: OdysseusAuthority) -> Pipeline:
    return Pipeline(make_registry(), mapping_table(), authority)


def openai_call(name: str, args: dict, call_id: str = "call_1") -> dict:
    return {
        "role": "assistant",
        "content": None,
        "tool_calls": [
            {"id": call_id, "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}
        ],
    }


def main() -> int:
    with tempfile.TemporaryDirectory() as ws:
        Path(ws, "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
        authority = OdysseusAuthority(ws, admin=True)
        if not authority.available:
            print("Odysseus is not importable (set ODYSSEUS_SRC); decide() would deny everything.")
            return 1
        pipe = make_pipeline(authority)
        cases = [
            ("read a workspace file", "openai", openai_call("read_file", {"path": "notes.txt"}, "c1")),
            ("escape the workspace", "openai", openai_call("read_file", {"path": "../../etc/passwd"}, "c2")),
            ("send email after reading (gate armed)", "openai", openai_call("send_email", {"to": "a@b.c", "subject": "s", "body": "b"}, "c3")),
            ("authorized, but not executed in 0.1", "openai", openai_call("get_workspace", {}, "c4")),
            ("text form (<tool_call><invoke>)", "odysseus_text", '<tool_call><invoke name="grep"><parameter name="pattern">hello</parameter></invoke></tool_call>'),
        ]
        for turn, (label, dialect, payload) in enumerate(cases):
            out = pipe.run_turn(dialect, "demo-model", payload, "demo-trace", turn)
            for res, rec in zip(out.results, out.observed):
                print(f"{label:42} -> {rec.stage:18} {res.status:18} execute_invoked={rec.execute_invoked}")
        return 0


if __name__ == "__main__":
    sys.exit(main())
