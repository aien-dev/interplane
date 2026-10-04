"""CrossAxis mapping table for Odysseus: explicit aliases plus passthrough for every native tool."""

from __future__ import annotations

from interplane.crossaxis import MappingTable

from .catalog import RUNTIME_ID, load_record
from .domains import CANONICAL

TABLE_VERSION = "1"


def mapping_table() -> MappingTable:
    """Alias rules for the canonical names, then passthrough for all recorded native names.

    ``catalog_digest`` is the recorded catalog's digest, so a table built against a different
    Odysseus catalog is rejected as ``stale_capability`` before the runtime is contacted.
    """
    record = load_record()
    rules = []
    for canon, native in CANONICAL.items():
        ns, _, name = canon.partition(".")
        rules.append(
            {
                "id": f"alias:{canon}",
                "kind": "alias",
                "from": {"namespace": ns, "name": name},
                "to": native,
            }
        )
    for cap in record["capabilities"]:
        name = cap["name"]
        rules.append(
            {
                "id": f"passthrough:{name}",
                "kind": "passthrough",
                "from": {"namespace": None, "name": name},
                "to": name,
            }
        )
    return MappingTable.from_dict(
        {
            "runtime": RUNTIME_ID,
            "table_version": TABLE_VERSION,
            "catalog_digest": record["catalog_digest"],
            "rules": rules,
        }
    )
