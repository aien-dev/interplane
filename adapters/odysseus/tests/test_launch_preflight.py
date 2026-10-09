"""Launch preflight (#93) against Odysseus's real code. No skips: the expectation follows whether
the host launch guard exists (a8c147b: yes; 2992bf6: no, so every workspace case is
``host_api_unsupported``)."""

import dataclasses
import importlib
import os
import sys
import types
import uuid
from pathlib import Path

import pytest

from interplane_adapter_odysseus.authority import NOT_EXECUTED, OdysseusAuthority
from interplane_adapter_odysseus.launch import PROCESS_TOOLS, REASON_CODES, LaunchPreflight, launch_preflight


def _has_guard():
    try:
        return hasattr(importlib.import_module("src.agent_runtime.process_resources"), "guard_launch_workspace")
    except Exception:  # noqa: BLE001
        return False


@pytest.fixture
def data(ody):
    return Path(importlib.import_module("src.constants").DATA_DIR).resolve()


@pytest.fixture
def home(data):
    return data.parent


@pytest.fixture
def sibling(home):
    d = home / f"ws-{uuid.uuid4().hex[:8]}"
    d.mkdir()
    (d / "f.txt").write_text("x", encoding="utf-8")
    return d


def _check(ody, ws, expect_code, expect_stage=None, tool="bash"):
    got = launch_preflight(ody, str(ws) if ws is not None else None, tool)
    if not _has_guard() and expect_code not in ("runtime_unavailable", "not_a_process_tool", "process_tool_unavailable"):
        expect_code, expect_stage = "host_api_unsupported", "runtime"
    assert got.code == expect_code, got
    if expect_stage:
        assert got.stage == expect_stage
    assert got.eligible is (expect_code == "structurally_eligible")
    _no_leak(got, ws)
    return got


def _no_leak(got, ws):
    text = " ".join(map(str, dataclasses.astuple(got)))
    assert got.code in REASON_CODES
    assert got.remediation and str(importlib.import_module("src.constants").DATA_DIR) not in text
    if ws:
        assert str(ws) not in text
        if "\0" not in str(ws):
            assert os.path.realpath(ws) not in text


def test_disjoint_sibling_is_eligible(ody, sibling):
    got = _check(ody, sibling, "structurally_eligible", "preflight")
    assert ("Odysseus still decides" in got.remediation) is got.eligible


def test_lookalike_name_is_not_rejected(ody, home):
    d = home / "data-lookalike"
    d.mkdir(exist_ok=True)
    _check(ody, d, "structurally_eligible")


def test_equals_data_dir(ody, data):
    _check(ody, data, "workspace_is_server_state", "selection")


def test_nested_in_data_dir(ody, data):
    d = data / "sub"
    d.mkdir(exist_ok=True)
    _check(ody, d, "workspace_within_server_state", "selection")


def test_ancestor_contains_data_dir(ody, home):
    _check(ody, home, "workspace_contains_server_state", "launch_boundary")


def test_ancestor_is_accepted_by_selection(ody, home):
    # Selection half of the #6651 disagreement; the guard half is test_ancestor_contains_data_dir.
    assert ody.tool_execution.vet_workspace(str(home)) == str(home)


def test_root_containing_another_protected_location(ody, sibling, monkeypatch):
    constants = importlib.import_module("src.constants")
    monkeypatch.setattr(constants, "SETTINGS_FILE", str(sibling / "settings.json"))
    _check(ody, sibling, "workspace_contains_server_state", "launch_boundary")


def test_symlink_alias_to_app_db(ody, data, sibling):
    target = Path(importlib.import_module("src.constants").APP_DB)
    assert target.exists()
    (sibling / "link").symlink_to(target)
    _check(ody, sibling, "workspace_aliases_server_state", "launch_boundary")


def test_hardlink_alias_to_app_db(ody, data, sibling):
    target = Path(importlib.import_module("src.constants").APP_DB)
    try:
        os.link(target, sibling / "hard")
    except OSError:
        pytest.fail("test layout must allow hardlinks (same filesystem)")
    _check(ody, sibling, "workspace_aliases_server_state", "launch_boundary")


def test_nonexistent_path(ody, home):
    _check(ody, home / "nope-does-not-exist", "workspace_invalid", "selection")


def test_embedded_null_byte_is_invalid_not_raised(ody, home):
    _check(ody, str(home) + "/bad\0name", "workspace_invalid", "selection")


def test_file_not_directory(ody, sibling):
    _check(ody, sibling / "f.txt", "workspace_invalid", "selection")


def test_missing_workspace(ody):
    _check(ody, None, "workspace_missing", "selection")
    _check(ody, "", "workspace_missing", "selection")


def test_inaccessible_directory(ody, home):
    d = home / f"locked-{uuid.uuid4().hex[:8]}"
    d.mkdir()
    d.chmod(0)
    try:
        assert os.geteuid() != 0, "tests must run as a non-root user"
        _check(ody, d, "workspace_inaccessible", "selection")
    finally:
        d.chmod(0o700)


def test_runtime_unavailable():
    auth = OdysseusAuthority(odysseus=False)
    got = auth.launch_preflight("/tmp", "bash")
    assert (got.code, got.stage, got.eligible) == ("runtime_unavailable", "runtime", False)
    assert launch_preflight(None, None).code == "runtime_unavailable"


@pytest.mark.parametrize("tool", ["read_file", "ls", "nope", "serve_start"])
def test_not_a_process_tool(ody, sibling, tool):
    got = _check(ody, sibling, "not_a_process_tool", "request", tool=tool)
    assert tool not in PROCESS_TOOLS and not got.eligible


def test_process_tool_missing(ody, sibling):
    stub = types.SimpleNamespace(TOOL_TAGS={}, TOOL_HANDLERS={})
    fake = dataclasses.replace(ody, agent_tools=stub)
    for tool in PROCESS_TOOLS:
        assert launch_preflight(fake, str(sibling), tool).code == "process_tool_unavailable"
    only_tags = dataclasses.replace(ody, agent_tools=types.SimpleNamespace(TOOL_TAGS={"bash": 1}, TOOL_HANDLERS={}))
    assert launch_preflight(only_tags, str(sibling), "bash").code == "process_tool_unavailable"


def test_host_api_missing(ody, sibling, monkeypatch):
    stub = types.ModuleType("src.agent_runtime.process_resources")  # no guard_launch_workspace
    monkeypatch.setitem(sys.modules, "src.agent_runtime.process_resources", stub)
    got = launch_preflight(ody, str(sibling), "bash")
    assert (got.code, got.stage, got.eligible) == ("host_api_unsupported", "runtime", False)


def test_guard_error_text_never_leaks_and_unknown_is_indeterminate(ody, sibling, monkeypatch):
    if not _has_guard():
        assert launch_preflight(ody, str(sibling), "bash").code == "host_api_unsupported"
        return
    pr = importlib.import_module("src.agent_runtime.process_resources")
    err = importlib.import_module("src.agent_runtime.resources").ResourceIdentityError

    def boom(root):
        raise err("Launch workspace cannot be inspected: " + str(root.path))

    monkeypatch.setattr(pr, "guard_launch_workspace", boom)
    got = launch_preflight(ody, str(sibling), "bash")
    assert got.code == "indeterminate" and str(sibling) not in " ".join(map(str, dataclasses.astuple(got)))

    def oserr(root):
        raise PermissionError("x")

    monkeypatch.setattr(pr, "guard_launch_workspace", oserr)
    assert launch_preflight(ody, str(sibling), "bash").code == "indeterminate"


def test_preflight_does_not_change_decide_or_execute(ody, workspace, run):
    before = [run("read_file", {"path": "notes.txt"}), run("bash", {"command": "echo hi"})]
    auth = OdysseusAuthority(str(workspace), admin=True)
    got = auth.launch_preflight()
    assert isinstance(got, LaunchPreflight) and got.code in REASON_CODES
    after = [run("read_file", {"path": "notes.txt"}), run("bash", {"command": "echo hi"})]
    for (a0, r0, c0), (a1, r1, c1) in zip(before, after):
        assert (r0.status, c0.decision, c0.execute_invoked) == (r1.status, c1.decision, c1.execute_invoked)
        assert (a0.decide_calls, a0.execute_calls) == (a1.decide_calls, a1.execute_calls)
    _, bash_res, _ = after[1]
    assert (bash_res.status, bash_res.error.message) == ("error", NOT_EXECUTED)


def test_eligible_preflight_never_executes(ody, sibling):
    auth = OdysseusAuthority(odysseus=ody)
    assert auth.launch_preflight(str(sibling), "bash").code in REASON_CODES
    assert not hasattr(auth, "execute_calls")
    assert sorted(p.name for p in sibling.iterdir()) == ["f.txt"]
