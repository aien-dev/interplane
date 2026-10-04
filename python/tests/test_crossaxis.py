import pytest

from interplane.core import ErrorCode, ToolRequest
from interplane.core import digest, jcs
from interplane.crossaxis import (
    BytesOnly,
    EstimatedTokens,
    ExpansionError,
    MappingError,
    MappingTable,
    coerce_arguments,
    discover,
    expand,
    measure,
    openai_tools_renderer,
    select,
    selection_digest,
    token_block,
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


# ---- measurement: units and the receipt shape


def test_select_receipt_omits_empty_optional_fields():
    cat = MockRuntime().catalog()
    sel, _ = select(cat, ["email"])
    d = sel.to_dict()
    assert "always_include" not in d and sel.selector == {"name": "domain_match", "version": "1"}
    assert d["selected"] == [{"name": "send_email", "rule_id": "domain:email", "domains": ["email"]}]
    sel2, _ = select(cat, [], None, ["fail_tool"])
    assert sel2.to_dict()["always_include"] == ["fail_tool"]


def test_measure_bytes_are_the_wire_bytes_and_no_null_token_fields():
    cat = MockRuntime().catalog()
    sel, caps = select(cat, ["email"], renderer=openai_tools_renderer)
    m = sel.measure
    assert m["rendered_bytes"] == {
        "full": len(jcs(openai_tools_renderer(cat.capabilities)).encode()),
        "selected": len(jcs(openai_tools_renderer(caps)).encode()),
    }
    assert m["full_rendered_bytes"] == m["rendered_bytes"]["full"]
    assert "tokens" not in m and "full_tokens" not in m and "tokenizer" not in m


MSGS = [{"role": "system", "content": "s"}, {"role": "user", "content": "find the readme"}]


def test_measure_estimated_tokens_exact():
    cat = MockRuntime().catalog()
    caps = [cat.get("send_email")]
    m = measure(cat.capabilities, caps, openai_tools_renderer, EstimatedTokens(), MSGS)
    t = m["tokens"]
    est = lambda v: -(-len(jcs(v).encode()) // 4)  # noqa: E731
    base = est(MSGS)
    assert t["unit"] == "tokens_estimated" and t["source"] == "estimator v1 ceil(utf8_bytes/4)"
    assert t["base_context"] == base
    full_t = est(openai_tools_renderer(cat.capabilities))
    assert t["tool_schema"] == {"full": full_t, "selected": est(openai_tools_renderer(caps))}
    assert t["first_turn_prompt"]["full"] == base + full_t
    assert "all_rounds_prompt" not in t and "completion" not in t


def test_estimator_and_bytes_parity_constants_match_rust():
    assert EstimatedTokens().count_prompt([], None) == 1
    assert BytesOnly().count_prompt([], {}) == 4


def test_measure_bytes_counter_and_bench_totals_passthrough():
    cat = MockRuntime().catalog()
    m = measure(
        cat.capabilities,
        cat.capabilities[:1],
        openai_tools_renderer,
        BytesOnly(),
        MSGS,
        all_rounds_prompt={"full": 9, "selected": 4},
        completion={"full": 3, "selected": 2},
    )
    t = m["tokens"]
    assert t["unit"] == "bytes" and t["tool_schema"]["full"] == m["rendered_bytes"]["full"]
    assert t["all_rounds_prompt"] == {"full": 9, "selected": 4}
    assert t["completion"] == {"full": 3, "selected": 2}


def test_token_block_unit_discipline():
    ok = dict(
        source="usage.prompt_tokens max_tokens=1 differential",
        base_context=10,
        tool_schema={"full": 5, "selected": 1},
        first_turn_prompt={"full": 15, "selected": 11},
    )
    assert token_block("tokens_model_reported", **ok)["unit"] == "tokens_model_reported"
    with pytest.raises(ValueError):
        token_block("tokens", **ok)
    with pytest.raises(ValueError):
        token_block("tokens_estimated", **{**ok, "tool_schema": {"full": -1, "selected": 0}})
    with pytest.raises(ValueError):
        token_block("tokens_estimated", **{**ok, "source": ""})
    with pytest.raises(ValueError):
        measure([], [], lambda c: [], EstimatedTokens())  # a counter needs the base messages


# ---- bounded expansion


def _email_selection(max_caps=None):
    cat = MockRuntime().catalog()
    return cat, select(cat, ["email"], max_caps)[0]


def test_expand_requested_excluded_adds_and_chains_digests():
    cat, sel = _email_selection()
    before = sel.to_dict()
    out = expand(sel, cat, {"kind": "requested_excluded", "name": "write_file"})
    assert sel.to_dict() == before, "input is not modified"
    assert [e["name"] for e in out.selected] == ["send_email", "write_file"]
    assert out.selected[1]["rule_id"] == "expansion:1"
    assert "write_file" not in [e["name"] for e in out.excluded]
    d = out.to_dict()
    assert d["expansions"] == [
        {
            "round": 1,
            "reason": "requested_excluded",
            "evidence_digest": digest({"kind": "requested_excluded", "name": "write_file"}),
            "added": ["write_file"],
            "refused": [],
        }
    ]
    assert d["parent_digest"] == selection_digest(sel) and d["selection_digest"] == selection_digest(d)
    assert d["selector"]["expansion"] == {"max_expansions": 2, "max_added_per_expansion": 8}
    assert "measure" not in d
    # deterministic
    again = expand(sel, cat, {"kind": "requested_excluded", "name": "write_file"})
    assert jcs(again.to_dict()) == jcs(d)
    # chaining: second expansion's parent is the first's digest
    out2 = expand(out, cat, {"kind": "requested_excluded", "name": "read_file"})
    assert out2.to_dict()["parent_digest"] == d["selection_digest"]
    assert [x["round"] for x in out2.to_dict()["expansions"]] == [1, 2]


def test_expand_drops_stale_measure_and_checks_catalog():
    cat = MockRuntime().catalog()
    sel, _ = select(cat, ["email"], renderer=openai_tools_renderer)
    assert sel.measure is not None
    out = expand(sel, cat, {"kind": "requested_excluded", "name": "read_file"})
    assert out.measure is None
    cat.capabilities.pop()
    cat.catalog_digest = None
    with pytest.raises(ExpansionError):
        expand(sel, cat, {"kind": "requested_excluded", "name": "read_file"})


def test_expand_refusal_reasons_and_order():
    cat, sel = _email_selection(max_caps=2)

    def refused(s, ev, **kw):
        return expand(s, cat, ev, **kw).to_dict()["expansions"][-1]["refused"]

    ev = {"kind": "discovery_hit", "names": ["nope", "send_email", "read_file", "write_file"]}
    out = expand(sel, cat, ev)
    last = out.to_dict()["expansions"][-1]
    assert last["added"] == ["read_file"]  # room for one under max_capabilities=2
    assert last["refused"] == [
        {"name": "nope", "reason": "not_in_catalog"},
        {"name": "send_email", "reason": "already_selected"},
        {"name": "write_file", "reason": "max_capabilities"},
    ]
    # max_added_per_expansion
    cat2, sel2 = _email_selection()
    last = expand(
        sel2, cat2, {"kind": "discovery_hit", "names": ["read_file", "list_dir", "web_fetch"]},
        max_added_per_expansion=2,
    ).to_dict()["expansions"][-1]
    assert last["added"] == ["read_file", "list_dir"]
    assert last["refused"] == [{"name": "web_fetch", "reason": "max_added_per_expansion"}]
    # max_expansions: a refused round does not count, but nothing more is added
    s = sel2
    for name in ("read_file", "list_dir"):
        s = expand(s, cat2, {"kind": "requested_excluded", "name": name})
    assert refused(s, {"kind": "requested_excluded", "name": "web_fetch"}) == [
        {"name": "web_fetch", "reason": "max_expansions"}
    ]
    s2 = expand(s, cat2, {"kind": "requested_excluded", "name": "web_fetch"})
    assert [e["round"] for e in s2.to_dict()["expansions"]] == [1, 2, 3]
    assert "web_fetch" not in [e["name"] for e in s2.selected]
    # recorded bounds win over later arguments
    assert refused(s, {"kind": "requested_excluded", "name": "web_fetch"}, max_expansions=99) == [
        {"name": "web_fetch", "reason": "max_expansions"}
    ]


def test_expand_siblings_and_bad_evidence():
    cat, sel = _email_selection()
    ev = {"kind": "requested_excluded", "name": "write_file", "include_domain_siblings": True}
    last = expand(sel, cat, ev).to_dict()["expansions"][-1]
    assert last["added"] == ["write_file", "append_note", "delete_file", "list_dir", "read_file"]
    for bad in ({}, {"kind": "x"}, {"kind": "requested_excluded"}, {"kind": "discovery_hit"}):
        with pytest.raises(ExpansionError):
            expand(sel, cat, bad)


def test_runtime_disabled_is_never_expanded_or_discovered():
    cat, sel = _email_selection()
    sel.excluded = [
        dict(e, reason="runtime_disabled") if e["name"] == "read_file" else e for e in sel.excluded
    ]
    assert "read_file" not in discover(cat, sel, "file")
    last = expand(sel, cat, {"kind": "requested_excluded", "name": "read_file"}).to_dict()
    assert last["expansions"][-1]["refused"] == [{"name": "read_file", "reason": "runtime_disabled"}]


def test_discover_is_ascii_case_insensitive_sorted_capped_and_excluded_only():
    cat, sel = _email_selection()
    assert discover(cat, sel, "FILE") == ["delete_file", "read_file", "write_file"]
    assert discover(cat, sel, "file", limit=2) == ["delete_file", "read_file"]
    assert discover(cat, sel, "email") == []  # send_email is selected, not excluded
    assert discover(cat, sel, "") == []
    assert discover(cat, sel, "fetch") == ["web_fetch"]


def test_expanded_capability_is_still_denied_by_the_runtime():
    from interplane.crossveil import default_pipeline

    pipe = default_pipeline()
    cat = pipe.runtime.catalog()
    sel, _ = select(cat, ["email"])
    out = expand(sel, cat, {"kind": "requested_excluded", "name": "write_file"})
    assert "write_file" in [e["name"] for e in out.selected]
    msg = {
        "role": "assistant",
        "content": None,
        "tool_calls": [
            {
                "id": "c1",
                "type": "function",
                "function": {"name": "write_file", "arguments": '{"path": "/a", "content": "x"}'},
            }
        ],
    }
    turn = pipe.run_turn("openai", "m", msg, "t", 0)
    assert [r.status for r in turn.results] == ["denied"]
    assert pipe.runtime.execute_calls == 0
