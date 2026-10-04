import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CONF = ROOT / "conformance" / "fixtures"
DIALECTS = ROOT / "dialects" / "fixtures"


@pytest.fixture
def envelope():
    def make(**over):
        env = {
            "interplane_version": "0.1",
            "message_id": "m-1",
            "trace_id": "t-1",
            "parent_id": None,
            "timestamp": "2026-01-01T00:00:00Z",
            "source": {"kind": "model", "id": "mock-model"},
            "destination": {"kind": "runtime", "id": "mock"},
            "payload": {
                "kind": "tool_request",
                "request_id": "r-1",
                "tool": {"namespace": None, "name": "read_file"},
                "arguments": {"path": "/tmp/a"},
                "provenance": {"dialect": "openai", "parser_version": "1.0.0"},
            },
        }
        env.update(over)
        return env

    return make


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))
