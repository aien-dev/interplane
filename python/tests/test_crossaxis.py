import pytest

from interplane.core import ErrorCode, ToolRequest
from interplane.crossaxis import (
    MappingError,
    MappingTable,
    coerce_arguments,
    measure,
    openai_tools_renderer,
    select,
)
from interplane.crossveil import MockRuntime, mock_mapping_table


def intent(ns, name, args=None, raw=None):
    return ToolRequest.from_dict(
        {
            "kind": "tool_request",
            "request_id": "r",
            "tool": {"namespace": ns, "name": name},
            "arguments": args or {},
            "provenance": {"dialect": "x", "parser_version": "1.0.0", "raw_name": raw},
        }
    )


TABLE = {
    "runtime": "rt",
    "table_version": "7",
    "rules": [
        {
            "id": "a1",
            "kind": "alias",
            "from": {"namespace": "fs", "name": "read"},
            "to": "read_file",
            "renames": [{"from": "path", "to": "file_path"}],
            "x_rule": 1,
        },
        {
            "id": "p1",
            "kind": "passthrough",
            "from": {"namespace": None, "name": "ping"},
            "to": "ping",
        },
    ],
    "x_table": True,
}


def test_mapping_alias_rename_passthrough():
    table = MappingTable.from_dict(TABLE)
    assert table.to_dict() == TABLE | {"rules": table.to_dict()["rules"]}
    req = table.map(intent("fs", "read", {"path": "/x", "n": 1}))
    assert (req.capability, req.arguments, req.runtime) == (
        "read_file",
        {"file_path": "/x", "n": 1},
        "rt",
    )
    assert req.mapping == {"table_version": "7", "rule_id": "a1", "passthrough": False}
    assert table.map(intent(None, "ping")).mapping["passthrough"] is True
    assert table.map(intent("my", "ping", raw="ping")).capability == "ping"


def test_mapping_is_exact_never_fuzzy():
    table = MappingTable.from_dict(TABLE)
    for ns, name in (
        (None, "read"),
        ("fs", "reads"),
        ("other", "read"),
        (None, "read_file"),
        ("fs", "ping"),
    ):
        with pytest.raises(MappingError) as e:
            table.map(intent(ns, name))
        assert e.value.code == ErrorCode.UNKNOWN_CAPABILITY


def test_mapping_rejects_bad_tables():
    for bad in (
        "x",
        {},
        {
            "runtime": "r",
            "table_version": "1",
            "rules": [{"id": "a", "kind": "fuzzy", "from": {"name": "x"}, "to": "y"}],
        },
        {"runtime": "r", "table_version": "1", "rules": [TABLE["rules"][1], TABLE["rules"][1]]},
    ):
        with pytest.raises(ValueError):
            MappingTable.from_dict(bad)


def test_mock_table():
    t = mock_mapping_table()
    assert t.map(intent("filesystem", "read")).mapping["rule_id"] == "alias:filesystem.read"
    assert t.map(intent(None, "read_file")).mapping["rule_id"] == "passthrough:read_file"
    assert t.map(intent("filesystem", "stat")).capability == "stat_file"
    with pytest.raises(MappingError):
        t.map(intent("nonexistent", "thing"))


def test_coercion():
    schema = {
        "properties": {
            "n": {"type": "integer"},
            "f": {"type": "number"},
            "b": {"type": "boolean"},
            "s": {"type": "string"},
            "u": {"type": ["integer", "null"]},
        }
    }
    args, coerced = coerce_arguments(
        {"n": "20", "f": "2.5", "b": "true", "s": "12", "u": "3", "z": "9"}, schema
    )
    assert args == {"n": 20, "f": 2.5, "b": True, "s": "12", "u": 3, "z": "9"}
    assert coerced == ["n", "f", "b", "u"]
    args, coerced = coerce_arguments({"n": "abc", "b": "yes", "f": "1e999x"}, schema)
    assert coerced == [] and args["n"] == "abc"
    assert coerce_arguments({"a": "1"}, None) == ({"a": "1"}, [])


def test_pipeline_records_coercion():
    from interplane.crossveil import default_pipeline

    pipe = default_pipeline()
    caps = pipe.runtime.catalog()
    assert caps.get("read_file") is not None


def test_select_and_measure():
    cat = MockRuntime().catalog()
    sel, caps = select(
        cat, ["filesystem"], max_capabilities=None, always_include=["slow_tool", "fail_tool"]
    )
    # pins first in catalog order, then domain matches sorted by name
    assert [c.name for c in caps] == [
        "fail_tool",
        "slow_tool",
        "append_note",
        "delete_file",
        "list_dir",
        "read_file",
        "write_file",
    ]
    assert [s["rule_id"] for s in sel.selected] == ["always_include"] * 2 + [
        "domain:filesystem"
    ] * 5
    reasons = {e["name"]: e["reason"] for e in sel.excluded}
    assert reasons == {
        "send_email": "domain_mismatch",
        "web_fetch": "domain_mismatch",
        "recall_memory": "domain_mismatch",
    }
    sel2, caps2 = select(cat, ["filesystem"], 3, ["fail_tool"], renderer=openai_tools_renderer)
    assert [c.name for c in caps2] == ["fail_tool", "append_note", "delete_file"]
    assert {e["name"]: e["reason"] for e in sel2.excluded}["read_file"] == "max_capabilities"
    # pins are never truncated, even beyond the limit
    _, caps3 = select(cat, ["filesystem"], 1, ["fail_tool", "slow_tool"])
    assert [c.name for c in caps3] == ["fail_tool", "slow_tool"]
    m = sel2.measure
    assert m["full_count"] == 10 and m["selected_count"] == 3
    assert m["selected_rendered_bytes"] < m["full_rendered_bytes"]
    assert sel2.catalog_digest == cat.computed_digest()
    assert measure(cat.capabilities, [], lambda c: "")["selected_rendered_bytes"] == 0


def test_select_no_domains():
    cat = MockRuntime().catalog()
    cat.capabilities[3].domains = []
    sel, caps = select(cat, ["filesystem"])
    assert {"name": "read_file", "reason": "no_domains"} in sel.excluded
    assert select(cat, ["filesystem"], None, ["read_file"])[1][0].name == "read_file"


def test_mapping_table_catalog_digest_and_messages():
    d = "sha256:" + "1" * 64
    table = MappingTable.from_dict(TABLE | {"catalog_digest": d})
    assert table.to_dict()["catalog_digest"] == d
    assert table.map(intent("fs", "read")).mapping["catalog_digest"] == d
    with pytest.raises(ValueError):
        MappingTable.from_dict(TABLE | {"catalog_digest": "nope"})
    with pytest.raises(MappingError) as e:
        table.map(intent("zz", "q"))
    assert e.value.message == "no mapping for tool: zz.q"
    with pytest.raises(MappingError) as e:
        table.map(intent(None, "q"))
    assert e.value.message == "no mapping for tool: q"
