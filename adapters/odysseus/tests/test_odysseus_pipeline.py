"""Through the real INTERPLANE Pipeline against Odysseus's own code. Skipped without Odysseus."""

import os

import pytest

from interplane.core import CapabilityRequest, ToolRef
from interplane.crossaxis import MappingTable
from interplane.crossveil import Pipeline

from interplane_adapter_odysseus import dialect, mapping_table
from interplane_adapter_odysseus.authority import NOT_EXECUTED, OdysseusAuthority
from interplane_adapter_odysseus.demo import make_pipeline, openai_call
from interplane_adapter_odysseus.catalog import tool_names


def test_catalog_tools_vs_odysseus_tool_tags(ody):
    # Finding at 2992bf6: tail_serve_output has a function schema (tool_schemas.py:899) and an
    # executor branch (tool_execution.py:1205) but is missing from TOOL_TAGS, so Odysseus's own
    # function_call_to_tool_block rejects it as an unknown function. Pinned so a fix is noticed.
    assert set(tool_names()) - set(ody.tool_tags) == {"tail_serve_output"}


def test_tool_missing_from_tool_tags_is_not_found(run):
    authority, res, rec = run("tail_serve_output", {"session_id": "s"})
    assert (res.status, rec.decision) == ("not_found", "not_found") and authority.execute_calls == 0


def test_authorized_read(run):
    authority, res, rec = run("read_file", {"path": "notes.txt"})
    assert (res.status, rec.stage, rec.decision) == ("ok", "SUCCEEDED", "authorized")
    assert res.data == {"output": "hello from the workspace\n", "exit_code": 0}
    prov = res.provenance
    assert (prov["content_kind"], prov["trust"], prov["trusted"]) == ("workspace_content", "workspace_untrusted", False)
    assert prov["runtime"] == "odysseus" and prov["capability"] == "read_file"
    decision = authority.decisions[res.request_id]
    assert decision.runtime_state == {"vocabulary": "odysseus.tool_effect", "values": ["read_workspace"]}
    assert (authority.decide_calls, authority.execute_calls) == (1, 1)


def test_trusted_workspace_flag(run):
    _, res, _ = run("read_file", {"path": "notes.txt"}, workspace_trusted=True)
    assert res.provenance["trust"] == "trusted_runtime"


@pytest.mark.parametrize(
    "name,args,expect",
    [("ls", {}, "notes.txt"), ("glob", {"pattern": "**/*.py"}, "code.py"), ("grep", {"pattern": "needle"}, "code.py:1:needle")],
)
def test_other_read_only_tools_execute(run, name, args, expect):
    _, res, rec = run(name, args)
    assert res.status == "ok" and expect in res.data["output"] and rec.execute_invoked


def test_canonical_alias_reaches_odysseus_tool(run):
    _, res, _ = run("filesystem.read", {"path": "notes.txt"})
    assert res.status == "ok" and res.provenance["capability"] == "read_file"


def test_mutation_after_external_context_requires_approval(run):
    authority, res, rec = run(
        "send_email", {"to": "a@b.c", "subject": "s", "body": "b"}, external_context_seen=True
    )
    assert (res.status, rec.stage, rec.execute_invoked) == ("requires_approval", "REQUIRES_APPROVAL", False)
    assert res.error.code == "approval_required"
    decision = authority.decisions[res.request_id]
    assert decision.approval is None  # no id minted: approval continuation is unsupported here
    assert decision.runtime_state["vocabulary"] == "odysseus.tool_gate"
    assert "External untrusted context" in decision.runtime_state["values"][0]
    assert authority.execute_calls == 0


def test_a_read_arms_the_gate_for_later_calls_in_the_same_trace(ody, workspace):
    authority = OdysseusAuthority(str(workspace), admin=True)
    pipe = make_pipeline(authority)
    first = pipe.run_turn("openai", "m", openai_call("read_file", {"path": "notes.txt"}, "c1"), "tr", 0)
    assert first.results[0].status == "ok"
    second = pipe.run_turn("openai", "m", openai_call("send_email", {"to": "a@b.c", "subject": "s", "body": "b"}, "c2"), "tr", 1)
    assert second.results[0].status == "requires_approval"
    other = pipe.run_turn("openai", "m", openai_call("send_email", {"to": "a@b.c", "subject": "s", "body": "b"}, "c3"), "other-trace", 0)
    # a different trace has its own gate: authorized (then "not executed"), not requires_approval
    assert other.observed[0].decision == "authorized"
    assert other.results[0].status == "error" and other.results[0].error.message == NOT_EXECUTED


def test_context_trust_arms_the_gate(ody):
    authority = OdysseusAuthority(admin=True)
    req = CapabilityRequest(
        request_id="r1", runtime="odysseus", capability="send_email",
        arguments={"to": "a@b.c", "subject": "s", "body": "b"}, tool=ToolRef(name="send_email"), mapping={},
    )
    assert authority.decide(req, {"trace_id": "a"}).decision == "authorized"
    assert authority.decide(req, {"trace_id": "b", "trust": "external_untrusted"}).decision == "requires_approval"
    assert authority.decide(req, {"trace_id": "c", "prior_trust": ["trusted_runtime", "workspace_untrusted"]}).decision == "requires_approval"
    assert authority.decide(req, {"trace_id": "d", "trust": "trusted_runtime"}).decision == "authorized"


def test_non_admin_blocked_tool_is_denied(run, ody):
    assert "grep" in ody.tool_security.NON_ADMIN_BLOCKED_TOOLS
    authority, res, rec = run("grep", {"pattern": "needle"}, admin=False)
    assert (res.status, rec.stage, rec.execute_invoked) == ("denied", "DENIED", False)
    assert res.error.code == "policy_denied" and "admin" in res.error.message
    assert authority.decisions[res.request_id].runtime_state["values"] == ["non_admin_blocked_tool"]
    assert authority.execute_calls == 0
    _, res2, _ = run("manage_tokens", {"action": "list"}, admin=False)
    assert res2.status == "denied"


def test_default_authority_is_non_admin(workspace, ody):
    assert OdysseusAuthority(str(workspace)).admin is False


def test_delegated_credential_refuses_privileged_tool(run):
    authority, res, rec = run("bash", {"command": "ls"}, delegated_credential=True)
    assert (res.status, rec.stage, rec.execute_invoked) == ("denied", "DENIED", False)
    assert "API-token" in res.error.message and authority.execute_calls == 0
    decision = authority.decisions[res.request_id]
    assert decision.approval is None and decision.runtime_state["values"] == ["delegated_credential"]


def test_nonexistent_tool_is_not_found_through_authority(ody, workspace):
    table = mapping_table().to_dict()
    table["rules"].append(
        {"id": "alias:filesystem.stat", "kind": "alias", "from": {"namespace": "filesystem", "name": "stat"}, "to": "stat_file"}
    )
    authority = OdysseusAuthority(str(workspace), admin=True)
    pipe = Pipeline(dialect.make_registry(), MappingTable.from_dict(table), authority)
    out = pipe.run_turn("openai", "m", openai_call("filesystem.stat", {"path": "a"}), "t", 0)
    res, rec = out.results[0], out.observed[0]
    assert (res.status, res.error.code, rec.decision) == ("not_found", "capability_not_found", "not_found")
    assert rec.decide_invoked and not rec.execute_invoked


def test_unmapped_name_never_reaches_the_runtime(run):
    authority, res, rec = run("frobnicate", {})
    assert (res.status, res.error.code) == ("rejected", "unknown_capability")
    assert authority.decide_calls == 0


def test_invalid_arguments_use_odysseus_message(run):
    authority, res, rec = run("read_file", {})
    assert (res.status, res.error.code, rec.decision) == ("rejected", "invalid_arguments", "invalid")
    assert "empty required arguments" in res.error.message and "read_file" in res.error.message
    assert authority.decide_calls == 1 and authority.execute_calls == 0


def test_malformed_arguments_rejected_before_runtime(ody, workspace):
    authority = OdysseusAuthority(str(workspace), admin=True)
    pipe = make_pipeline(authority)
    bad = {"role": "assistant", "content": None, "tool_calls": [{"id": "c", "type": "function", "function": {"name": "read_file", "arguments": "{not json"}}]}
    out = pipe.run_turn("openai", "m", bad, "t", 0)
    assert out.results[0].error.code == "malformed_tool_call" and not out.observed[0].decide_invoked


@pytest.mark.parametrize("path", ["../outside.txt", "/etc/passwd", "sub/../../outside.txt"])
def test_path_escape_is_denied_and_never_executed(run, path):
    authority, res, rec = run("read_file", {"path": path})
    assert (res.status, rec.stage) == ("denied", "DENIED")
    assert "outside the workspace" in res.error.message and "SECRET" not in str(res.to_dict())
    assert authority.execute_calls == 0 and not rec.execute_invoked
    assert str(authority.workspace) not in res.error.message  # reasons are redacted


def test_symlink_escape_is_denied(run, workspace, tmp_path):
    (workspace / "link.txt").symlink_to(tmp_path / "outside.txt")
    authority, res, _ = run("read_file", {"path": "link.txt"})
    assert res.status == "denied" and authority.execute_calls == 0


@pytest.mark.parametrize("name,args", [("ls", {"path": ".."}), ("glob", {"pattern": "*", "path": "/etc"}), ("grep", {"pattern": "x", "path": "../"})])
def test_search_tool_escape_is_denied(run, name, args):
    authority, res, _ = run(name, args)
    assert res.status == "denied" and authority.execute_calls == 0


def test_sensitive_path_inside_workspace_is_denied(run, workspace):
    (workspace / ".ssh").mkdir()
    (workspace / ".ssh" / "id_rsa").write_text("k", encoding="utf-8")
    authority, res, _ = run("read_file", {"path": ".ssh/id_rsa"})
    assert res.status == "denied" and authority.execute_calls == 0


def test_no_workspace_means_refused(run):
    authority, res, _ = run("read_file", {"path": "notes.txt"}, workspace_arg=False)
    assert res.status == "denied" and authority.execute_calls == 0


def test_execution_error_path(run):
    authority, res, rec = run("read_file", {"path": "missing.txt"})
    assert (res.status, res.error.code, rec.stage) == ("error", "execution_error", "FAILED")
    assert "not found" in res.error.message and rec.execute_invoked
    assert str(authority.workspace) not in res.error.message


def test_other_authorized_tools_are_not_executed(run):
    authority, res, rec = run("get_workspace", {})
    assert rec.decision == "authorized" and rec.execute_invoked
    assert (res.status, res.error.code, res.error.message) == ("error", "execution_error", NOT_EXECUTED)
    _, res2, _ = run("ask_user", {"question": "q", "options": ["a", "b"]})
    assert res2.status == "error" and res2.error.message == NOT_EXECUTED


def test_text_dialect_through_pipeline(ody, workspace):
    authority = OdysseusAuthority(str(workspace), admin=True)
    pipe = make_pipeline(authority)
    forms = [
        '<tool_call><invoke name="read_file"><parameter name="path">notes.txt</parameter></invoke></tool_call>',
        '```read_file\nnotes.txt\n```',
        '[TOOL_CALL]{tool => "read_file", args => {--path "notes.txt"}}[/TOOL_CALL]',
    ]
    for turn, text in enumerate(forms):
        out = pipe.run_turn("odysseus_text", "m", text, "t", turn)
        assert out.results[0].status == "ok", text
        assert "hello from the workspace" in out.rendered[0] and out.rendered[0].startswith("### read_file: notes.txt")


def test_text_dialect_agrees_with_odysseus_parse_tool_blocks(ody):
    from src.tool_parsing import parse_tool_blocks

    cases = {
        '<tool_call><invoke name="bash"><parameter name="command">ls -la</parameter></invoke></tool_call>': ("bash", "ls -la"),
        "```bash\nwhoami\n```": ("bash", "whoami"),
        '[TOOL_CALL]{tool => "bash", args => {--command "ls"}}[/TOOL_CALL]': ("bash", "ls"),
    }
    for text, (name, content) in cases.items():
        theirs = parse_tool_blocks(text)
        mine = dialect.parse(text, "m", "t", 0).intents
        assert [(b.tool_type, b.content) for b in theirs] == [(name, content)]
        assert [(i.tool.name, i.arguments["command"]) for i in mine] == [(name, content)]


def test_render_matches_odysseus_format_tool_result(ody, run):
    _, res, _ = run("read_file", {"path": "notes.txt"})
    real = ody.tool_execution.format_tool_result("read_file: notes.txt", res.data)
    intent = dialect.parse("```read_file\nnotes.txt\n```", "m", "t", 0).intents[0]
    assert dialect.render_result(res, intent) == real
    err = {"error": "x is missing", "exit_code": 1}
    errres = {"status": "error", "error": {"code": "execution_error", "message": "x is missing"}}
    assert dialect.render_result(errres, intent) == ody.tool_execution.format_tool_result("read_file: notes.txt", err)


def test_round_message_uses_odysseus_wrapper(ody):
    msg = dialect.round_message(["### a\nout"], ody)
    assert msg["role"] == "user" and "<<<UNTRUSTED_SOURCE_DATA>>>" in msg["content"]
    assert msg["metadata"]["trusted"] is False


def test_vet_workspace_refuses_unusable_roots(ody, tmp_path):
    with pytest.raises(ValueError):
        OdysseusAuthority(str(tmp_path / "does-not-exist"))
    with pytest.raises(ValueError):
        OdysseusAuthority("/")


def test_importing_odysseus_does_not_write_inside_the_checkout(ody):
    import src.constants

    assert not os.path.realpath(src.constants.DATA_DIR).startswith(os.path.realpath(ody.root))


def test_system_result_integrity_maps_to_a_trust_level(ody, workspace):
    # Odysseus ResultIntegrity.SYSTEM (tool_capabilities.py:37-46 at 2992bf6) is not a
    # TrustLevel. It is Odysseus's label for server-authored output, so it maps to
    # trusted_runtime, unless Odysseus's own tool_result_should_arm_gate (:506-528) says the
    # producer marked this result untrusted_content; then external_untrusted.
    a = OdysseusAuthority(str(workspace), admin=True)
    assert ody.tool_capabilities.capabilities_for_action("update_plan", "").result_integrity.value == "system"
    assert a._integrity("update_plan", "") == "trusted_runtime"
    assert a._integrity("update_plan", "", {"ok": True}) == "trusted_runtime"
    assert a._integrity("update_plan", "", {"untrusted_content": True}) == "external_untrusted"
    assert a._integrity("read_file", "") == "workspace_untrusted"
    assert a._integrity("no_such_tool", "") == "external_untrusted"
