import os
import sys
import tempfile
from pathlib import Path

import pytest

# A controlled layout for the launch-preflight tests: DATA_DIR is <home>/data, so sibling directories
# of it are disjoint workspaces. Must be set before Odysseus is first imported; an operator value wins.
_home = Path(tempfile.mkdtemp(prefix="interplane-odysseus-home-")).resolve()
(_home / "data").mkdir()
os.environ.setdefault("ODYSSEUS_DATA_DIR", str(_home / "data"))

ADAPTER = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADAPTER))

from interplane_adapter_odysseus import _odysseus  # noqa: E402
from interplane_adapter_odysseus.authority import OdysseusAuthority  # noqa: E402
from interplane_adapter_odysseus.demo import make_pipeline, openai_call, register_user_turn  # noqa: E402


@pytest.fixture(scope="session")
def ody():
    found = _odysseus.load()
    if found is None:
        pytest.skip(f"Odysseus not importable (set {_odysseus.ENV} to a checkout)")
    return found


@pytest.fixture
def workspace(tmp_path):
    ws = tmp_path / "ws"
    (ws / "sub").mkdir(parents=True)
    (ws / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
    (ws / "sub" / "code.py").write_text("needle = 1\n", encoding="utf-8")
    (tmp_path / "outside.txt").write_text("SECRET\n", encoding="utf-8")
    return ws


class CountingAuthority(OdysseusAuthority):
    """Counts the two Crossveil callbacks so tests can assert the runtime never executed."""

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self.decide_calls = 0
        self.execute_calls = 0

    def decide(self, req, ctx):
        self.decide_calls += 1
        return super().decide(req, ctx)

    def execute(self, req, decision, ctx):
        self.execute_calls += 1
        return super().execute(req, decision, ctx)


@pytest.fixture
def run(ody, workspace):
    """run(name, args, **authority_kw) -> (authority, result, record), on a fresh trace."""

    def _run(name, args, workspace_arg=True, **kw):
        kw.setdefault("admin", True)
        authority = CountingAuthority(str(workspace) if workspace_arg else None, **kw)
        pipe = make_pipeline(authority)
        register_user_turn(pipe, "trace-x", "test request")
        out = pipe.run_turn("openai", "test-model", openai_call(name, args), "trace-x", 0)
        return authority, out.results[0], out.observed[0]

    return _run
