"""INTERPLANE reference adapter for Odysseus (AGPL-3.0-or-later).

Odysseus keeps all authority. This package translates Odysseus's own decisions into INTERPLANE
objects; it never changes Odysseus and never authorizes anything itself.
"""

from .authority import OdysseusAuthority
from .catalog import load_catalog, load_record
from .dialect import make_registry
from .domains import CANONICAL, DOMAIN_SET, TOOL_DOMAINS, catalog_with_domains
from .mapping import mapping_table

__all__ = [
    "CANONICAL", "DOMAIN_SET", "OdysseusAuthority", "TOOL_DOMAINS", "catalog_with_domains",
    "load_catalog", "load_record", "make_registry", "mapping_table",
]
