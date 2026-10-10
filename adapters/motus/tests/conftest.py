import sys
from pathlib import Path

import pytest

ADAPTER = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADAPTER))

from interplane.core import Decision  # noqa: E402
from interplane.crossveil import MockRuntime, mock_mapping_table  # noqa: E402

from interplane_adapter_motus.gate import Gate  # noqa: E402
from interplane_adapter_motus.local_authority import LocalReadOnlyAuthority, build_catalog  # noqa: E402
from interplane_adapter_motus.mapping import mapping_for  # noqa: E402


@pytest.fixture
def workspace(tmp_path):
    ws = tmp_path / "ws"
    (ws / "sub").mkdir(parents=True)
    (ws / "notes.txt").write_text("hello from the workspace\n", encoding="utf-8")
    (tmp_path / "outside.txt").write_text("SECRET\n", encoding="utf-8")
    return ws


@pytest.fixture
def make_local(workspace):
    """make_local(**authority_kw) -> (authority, gate) over the local stand-in authority."""

    def _make(**kw):
        authority = LocalReadOnlyAuthority(str(workspace), **kw)
        gate = Gate(authority, mapping_for(build_catalog(), authority.runtime_id))
        gate.register_user_turn("test request")
        return authority, gate

    return _make


@pytest.fixture
def make_mock():
    """make_mock(runtime=None, table="mock-table") -> (runtime, gate) over the Core mock runtime."""

    def _make(runtime=None, table="mock-table"):
        runtime = runtime or MockRuntime()
        gate = Gate(runtime, mock_mapping_table(table))
        gate.register_user_turn("test request")
        return runtime, gate

    return _make


def mock_approver(approved="authorized"):
    """Plays the mock runtime's own continuation answer (as bench/conformance does)."""

    def approver(pending):
        return Decision(
            request_id=pending.request_id,
            decision=approved,
            capability=pending.capability_request.capability,
            authority={"runtime": "mock", "policy_engine": "mock.policy", "decision_id": None},
            approval={"approval_id": pending.approval_id, "scope": "single_action", "expires_at": None},
        )

    return approver
