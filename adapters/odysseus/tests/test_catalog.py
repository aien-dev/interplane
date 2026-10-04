from interplane.core import Catalog
from interplane.crossaxis import MappingTable
from interplane.core import ToolRequest

from interplane_adapter_odysseus import (
    CANONICAL, DOMAIN_SET, TOOL_DOMAINS, catalog_with_domains, load_catalog, load_record, mapping_table,
)
from interplane_adapter_odysseus.catalog import tool_names


def test_recorded_catalog_has_71_tools():
    record = load_record()
    assert record["runtime"] == "odysseus" and record["catalog_version"] == "2992bf6"
    assert len(record["capabilities"]) == 71
    assert isinstance(load_catalog(), Catalog)


def test_every_catalog_tool_has_at_least_one_domain():
    for cap in catalog_with_domains().capabilities:
        assert cap.domains, f"{cap.name} has no domain"
        assert set(cap.domains) <= DOMAIN_SET, cap.name


def test_domain_table_matches_catalog_exactly():
    assert set(TOOL_DOMAINS) == set(tool_names())


def test_domain_set_is_the_fixed_set():
    assert DOMAIN_SET == {
        "filesystem", "code", "process", "web", "email", "calendar", "contacts", "documents",
        "notes", "tasks", "memory", "sessions", "models", "serving", "image", "admin", "ui",
        "research", "user",
    }
    assert set().union(*TOOL_DOMAINS.values()) <= DOMAIN_SET


def test_canonical_aliases():
    assert len(CANONICAL) == 16
    by_native = {c.name: c for c in catalog_with_domains().capabilities}
    for canon, native in CANONICAL.items():
        ns, _, name = canon.partition(".")
        ref = by_native[native].canonical
        assert (ref.namespace, ref.name) == (ns, name)
    assert sum(1 for c in by_native.values() if c.canonical) == 16


def test_digest_and_effects_preserved():
    record, cat = load_record(), catalog_with_domains()
    assert cat.catalog_digest == record["catalog_digest"]
    for rec, cap in zip(record["capabilities"], cat.capabilities):
        assert rec["name"] == cap.name and rec["runtime_effects"] == cap.runtime_effects
        assert rec["schema_digest"] == cap.schema_digest


def test_mapping_table_aliases_and_passthrough():
    table = mapping_table()
    assert isinstance(table, MappingTable) and table.runtime == "odysseus"
    assert table.catalog_digest == load_record()["catalog_digest"]
    kinds = [r.kind for r in table.rules]
    assert kinds.count("alias") == 16 and kinds.count("passthrough") == 71
    assert {r.to for r in table.rules if r.kind == "passthrough"} == set(tool_names())


def _req(ns, name):
    from interplane.lenshift._common import split_name

    raw = f"{ns}.{name}" if ns else name
    return ToolRequest(
        request_id="r1", tool=split_name(raw), arguments={}, provenance={"raw_name": raw}
    )


def test_mapping_resolves_every_alias_and_native_name():
    table = mapping_table()
    for canon, native in CANONICAL.items():
        cap = table.map(_req(*canon.split("."))).capability
        assert cap == native
    for name in tool_names():
        assert table.map(_req(None, name)).capability == name
