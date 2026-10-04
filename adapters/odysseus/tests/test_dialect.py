import json

from interplane.lenshift import default_registry
from interplane.lenshift import qwen35

from interplane_adapter_odysseus import dialect
from interplane_adapter_odysseus.dialect import make_registry

# Shapes copied from odysseus tests/test_redos_xml_tool_parsers.py (lines 49, 99, 111-114).
XML = '<tool_call><invoke name="bash"><parameter name="command">ls -la</parameter></invoke></tool_call>'
FENCED = '```bash\nwhoami\n```'
FENCED_JSON = '```web_search\n{"query": "weather today", "time_filter": "week"}\n```'
FENCED_INVOKE = '```python\n<invoke name="bash"><parameter name="command">whoami</parameter></invoke>\n```'
TOOL_CALL = '[TOOL_CALL]{tool => "bash", args => {--command "ls"}}[/TOOL_CALL]'
TOOL_CALL_SHELL = '[TOOL_CALL]{tool => "shell", args => {--command "ls"}}[/TOOL_CALL]'


def parse(text):
    return dialect.parse(text, "m", "t", 0)


def test_xml_invoke_form():
    turn = parse("before " + XML + " after")
    (intent,) = turn.intents
    assert intent.tool.name == "bash" and intent.arguments == {"command": "ls -la"}
    assert intent.provenance["dialect"] == "odysseus_text" and not turn.rejected
    assert turn.text == "before  after"


def test_xml_multiple_parameters_and_calls():
    text = (
        '<tool_call><invoke name="web_search"><parameter name="query">weather</parameter>'
        '<parameter name="time_filter">week</parameter></invoke>'
        '<invoke name="read_file"><parameter name="path">a.txt</parameter></invoke></tool_call>'
    )
    turn = parse(text)
    assert [i.tool.name for i in turn.intents] == ["web_search", "read_file"]
    assert turn.intents[0].arguments == {"query": "weather", "time_filter": "week"}


def test_fenced_forms():
    (bash,) = parse(FENCED).intents
    assert (bash.tool.name, bash.arguments) == ("bash", {"command": "whoami"})
    (ws,) = parse(FENCED_JSON).intents
    assert ws.arguments == {"query": "weather today", "time_filter": "week"}
    (inv,) = parse(FENCED_INVOKE).intents
    assert (inv.tool.name, inv.arguments) == ("bash", {"command": "whoami"})
    (rf,) = parse("```read_file\nnotes.txt\nignored second line\n```").intents
    assert rf.arguments == {"path": "notes.txt"}


def test_fence_metadata_and_other_languages_are_prose():
    for text in ('```bash {title="setup"}\nls\n```', "```text\nhello\n```", "```\nplain\n```"):
        turn = parse(text)
        assert not turn.intents and not turn.rejected, text
        assert turn.text


def test_tool_call_block_form():
    (intent,) = parse(TOOL_CALL).intents
    assert (intent.tool.name, intent.arguments) == ("bash", {"command": "ls"})
    (js,) = parse('[TOOL_CALL]{tool => "read_file", args => {"path": "a.txt"}}[/TOOL_CALL]').intents
    assert js.arguments == {"path": "a.txt"}
    (nested,) = parse('[TOOL_CALL]{tool => "bash", args => {--command "echo {x} done"}}[/TOOL_CALL]').intents
    assert nested.arguments == {"command": "echo {x} done"}


def test_lenshift_never_renames_shell():
    # Odysseus maps "shell" to bash via its own alias map; Lenshift keeps the raw name.
    (intent,) = parse(TOOL_CALL_SHELL).intents
    assert intent.tool.name == "shell" and intent.provenance["raw_name"] == "shell"


def test_truncated_and_malformed_calls_are_rejected_never_guessed():
    for text in (
        '<tool_call><invoke name="bash"><parameter name="command">ls',
        '<tool_call><invoke name="bash"><parameter name="command">ls</parameter>',
        "[TOOL_CALL]{tool => \"bash\", args => {--command \"ls\"}}",
        "```bash\nls",
    ):
        turn = parse(text)
        assert not turn.intents and turn.partial and turn.rejected[0]["message"] == "truncated tool call", text
    dup = parse('<tool_call><invoke name="x"><parameter name="a">1</parameter><parameter name="a">2</parameter></invoke></tool_call>')
    assert dup.rejected[0]["message"] == "duplicate parameter: a"


def test_hermes_json_is_left_to_qwen35():
    hermes = '<tool_call>\n{"name": "bash", "arguments": {"command": "mkdir -p agent-test"}}\n</tool_call>'
    mine = parse(hermes)
    assert not mine.intents and "qwen35" in mine.rejected[0]["message"]
    theirs = qwen35.parse(hermes, "m", "t", 0)
    assert theirs.intents[0].tool.name == "bash"
    assert theirs.intents[0].provenance["dialect_version"] == "hermes_json"


def test_registry_is_local_and_core_is_untouched():
    assert "odysseus_text" in make_registry().names()
    assert "odysseus_text" not in default_registry.names()
    assert make_registry().names() == ["odysseus_text", "openai", "qwen35"]


def test_render_result_text_mode():
    ok = {"kind": "result", "status": "ok", "data": {"output": "hello\n", "exit_code": 0}, "request_id": "r", "error": None}
    turn = parse(FENCED_JSON.replace("web_search", "read_file").replace('{"query": "weather today", "time_filter": "week"}', '{"path": "notes.txt"}'))
    text = dialect.render_result(ok, turn.intents[0])
    assert text == "### read_file: notes.txt\n```\nhello\n\n```"
    denied = {"status": "denied", "error": {"code": "policy_denied", "message": "nope"}}
    assert dialect.render_result(denied, turn.intents[0]) == "### read_file: BLOCKED\n**Error:** nope"
    json.dumps(text)
