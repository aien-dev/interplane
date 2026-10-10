"""Mapping table for a catalog: passthrough for every advertised capability, pinned to its digest."""

from __future__ import annotations

from interplane.core import Catalog
from interplane.crossaxis import MappingTable

TABLE_VERSION = "1"


def mapping_for(catalog: Catalog, runtime_id: str) -> MappingTable:
    """Passthrough rules only, pinned to the catalog digest the model was shown.

    If the authority's catalog later changes, the Pipeline refuses every call as
    ``stale_capability`` before the authority is asked anything.
    """
    digest = catalog.catalog_digest or catalog.computed_digest()
    rules = [
        {
            "id": f"passthrough:{cap.name}",
            "kind": "passthrough",
            "from": {"namespace": None, "name": cap.name},
            "to": cap.name,
        }
        for cap in catalog.capabilities
    ]
    return MappingTable.from_dict(
        {
            "runtime": runtime_id,
            "table_version": TABLE_VERSION,
            "catalog_digest": digest,
            "rules": rules,
        }
    )
