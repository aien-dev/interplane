"""CrossAxis: explicit capability mapping and deterministic domain selection.

CrossAxis answers "which runtime capability is this canonical tool?" and "which capabilities
should this model see?". It never answers "may the model run it?".
"""

import copy
import re
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

from .core import (
    DIGEST_RE,
    Catalog,
    CapabilityRequest,
    ErrorCode,
    Selection,
    ToolRef,
    ToolRequest,
    jcs,
)


class MappingError(Exception):
    """Mapping refused. ``code`` is an ErrorCode (``unknown_capability`` for unmapped names)."""

    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


@dataclass
class Rule:
    id: str
    kind: str
    namespace: Optional[str]
    name: str
    to: str
    renames: list = field(default_factory=list)  # [(from, to)]
    extra: dict = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Any) -> "Rule":
        if not isinstance(d, dict):
            raise ValueError("mapping rule must be an object")
        rid, kind, to = d.get("id"), d.get("kind"), d.get("to")
        frm = d.get("from")
        if not isinstance(rid, str) or not rid:
            raise ValueError("mapping rule needs an id")
        if kind not in ("alias", "passthrough"):
            raise ValueError(f"rule {rid}: kind must be alias or passthrough")
        if not isinstance(frm, dict) or not isinstance(frm.get("name"), str):
            raise ValueError(f"rule {rid}: from.name is required")
        if not isinstance(to, str) or not to:
            raise ValueError(f"rule {rid}: to is required")
        renames = []
        for r in d.get("renames") or []:
            if not (
                isinstance(r, dict)
                and isinstance(r.get("from"), str)
                and isinstance(r.get("to"), str)
            ):
                raise ValueError(f"rule {rid}: bad rename")
            renames.append((r["from"], r["to"]))
        known = {"id", "kind", "from", "to", "renames"}
        extra = {k: copy.deepcopy(v) for k, v in d.items() if k not in known}
        return cls(rid, kind, frm.get("namespace"), frm["name"], to, renames, extra)

    def to_dict(self) -> dict:
        out = {
            "id": self.id,
            "kind": self.kind,
            "from": {"namespace": self.namespace, "name": self.name},
            "to": self.to,
        }
        if self.renames:
            out["renames"] = [{"from": a, "to": b} for a, b in self.renames]
        out.update(copy.deepcopy(self.extra))
        return out


@dataclass
class MappingTable:
    runtime: str
    table_version: str
    rules: list
    catalog_digest: Optional[str] = None
    extra: dict = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Any) -> "MappingTable":
        if not isinstance(d, dict):
            raise ValueError("mapping table must be an object")
        runtime, version = d.get("runtime"), d.get("table_version")
        if not isinstance(runtime, str) or not runtime or not isinstance(version, (str, int)):
            raise ValueError("mapping table needs runtime and table_version")
        rules = [Rule.from_dict(r) for r in d.get("rules", [])]
        ids = [r.id for r in rules]
        if len(set(ids)) != len(ids):
            raise ValueError("duplicate rule id")
        cat = d.get("catalog_digest")
        if cat is not None and not (isinstance(cat, str) and DIGEST_RE.fullmatch(cat)):
            raise ValueError("catalog_digest must be a sha256 digest")
        known = ("runtime", "table_version", "rules", "catalog_digest")
        extra = {k: copy.deepcopy(v) for k, v in d.items() if k not in known}
        return cls(runtime, str(version), rules, cat, extra)

    def to_dict(self) -> dict:
        out = {
            "runtime": self.runtime,
            "table_version": self.table_version,
            "rules": [r.to_dict() for r in self.rules],
        }
        if self.catalog_digest is not None:
            out["catalog_digest"] = self.catalog_digest
        out.update(copy.deepcopy(self.extra))
        return out

    def lookup(self, intent: ToolRequest) -> Optional[Rule]:
        """Exact, deterministic: aliases by (namespace, name), then passthrough by bare name."""
        ns, name = intent.tool.namespace, intent.tool.name
        for rule in self.rules:
            if rule.kind == "alias" and rule.namespace == ns and rule.name == name:
                return rule
        raw = intent.provenance.get("raw_name") if isinstance(intent.provenance, dict) else None
        bare = {name} if ns is None else set()
        if isinstance(raw, str):
            bare.add(raw)
        for rule in self.rules:
            if rule.kind == "passthrough" and rule.name in bare:
                return rule
        return None

    def map(self, intent: ToolRequest) -> CapabilityRequest:
        rule = self.lookup(intent)
        ns, name = intent.tool.namespace, intent.tool.name
        if rule is None:
            label = f"{ns}.{name}" if ns else name
            raise MappingError(ErrorCode.UNKNOWN_CAPABILITY, f"no mapping for tool: {label}")
        args = dict(intent.arguments)
        for src, dst in rule.renames:
            if src in args:
                if dst in args and dst != src:
                    raise MappingError(
                        ErrorCode.INVALID_ARGUMENTS, f"rename target {dst!r} already present"
                    )
                args[dst] = args.pop(src)
        mapping = {
            "table_version": self.table_version,
            "rule_id": rule.id,
            "passthrough": rule.kind == "passthrough",
        }
        if self.catalog_digest is not None:
            mapping["catalog_digest"] = self.catalog_digest
        tool = ToolRef(name=intent.tool.name, namespace=intent.tool.namespace).keep_null(
            "namespace"
        )
        return CapabilityRequest(
            request_id=intent.request_id,
            runtime=self.runtime,
            capability=rule.to,
            arguments=args,
            tool=tool,
            mapping=mapping,
        )


_INT_RE = re.compile(r"[+-]?[0-9]+")
_FLOAT_RE = re.compile(r"[+-]?([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?")


def coerce_arguments(args: dict, parameters_schema: Optional[dict]) -> tuple:
    """Coerce string values to the integer/number/boolean the schema declares.

    Returns ``(new_args, coerced_keys)``. A string is only coerced when the schema does not accept
    strings for that key and the text parses; everything else is left for the runtime to judge.
    """
    props = (parameters_schema or {}).get("properties")
    out = dict(args)
    coerced: list = []
    if not isinstance(props, dict):
        return out, coerced
    for key, value in args.items():
        spec = props.get(key)
        if not isinstance(value, str) or not isinstance(spec, dict):
            continue
        types = spec.get("type")
        types = [types] if isinstance(types, str) else list(types or [])
        if "string" in types:
            continue
        text = value.strip()
        new: Any = value
        if "integer" in types and _INT_RE.fullmatch(text):
            new = int(text)
        elif "number" in types and _INT_RE.fullmatch(text):
            new = int(text)
        elif "number" in types and _FLOAT_RE.fullmatch(text):
            new = float(text)
        elif "boolean" in types and text in ("true", "false"):
            new = text == "true"
        if new is not value:
            out[key] = new
            coerced.append(key)
    return out, coerced


def select(
    catalog: Catalog,
    requested_domains: list,
    max_capabilities: Optional[int] = None,
    always_include: Optional[list] = None,
    renderer: Optional[Callable] = None,
) -> tuple:
    """Selector ``domain_match`` v1. Returns ``(Selection, [CapabilityDescriptor])``.

    Pinned names come first (rule id ``always_include``, catalog order, never truncated). Then
    capabilities with a requested domain, sorted by name (rule id ``domain:<matched domain>``, the
    first matching domain in the capability's own order). Capabilities without domains are excluded
    as ``no_domains``, the rest as ``domain_mismatch``. When the total exceeds ``max_capabilities``
    the domain-selected entries are dropped from the end of the sorted list (``max_capabilities``).
    """
    wanted = set(requested_domains)
    pinned = set(always_include or [])
    pins = [(c, "always_include") for c in catalog.capabilities if c.name in pinned]
    matched: list = []
    excluded: list = []
    for cap in sorted(catalog.capabilities, key=lambda c: c.name):
        if cap.name in pinned:
            continue
        domains = cap.domains or []
        hit = next((d for d in domains if d in wanted), None)
        if hit is not None:
            matched.append((cap, f"domain:{hit}"))
        elif not domains:
            excluded.append({"name": cap.name, "reason": "no_domains"})
        else:
            excluded.append({"name": cap.name, "reason": "domain_mismatch"})
    if max_capabilities is not None:
        room = max(max_capabilities - len(pins), 0)
        for cap, _ in matched[room:]:
            excluded.append({"name": cap.name, "reason": "max_capabilities"})
        matched = matched[:room]
    excluded.sort(key=lambda e: e["name"])
    chosen = pins + matched
    selected_caps = [c for c, _ in chosen]
    selection = Selection(
        runtime=catalog.runtime,
        catalog_digest=catalog.catalog_digest or catalog.computed_digest(),
        selector={"name": "domain_match", "version": "1", "max_capabilities": max_capabilities},
        requested_domains=list(requested_domains),
        always_include=list(always_include or []),
        selected=[
            {"name": c.name, "rule_id": rule, "domains": list(c.domains or [])}
            for c, rule in chosen
        ],
        excluded=excluded,
    )
    if renderer is not None:
        selection.measure = measure(catalog.capabilities, selected_caps, renderer)
    return selection, selected_caps


def openai_tools_renderer(caps: list) -> list:
    """Render descriptors as OpenAI-style ``tools`` entries (the default measuring renderer)."""
    return [
        {
            "type": "function",
            "function": {"name": c.name, "description": c.description, "parameters": c.parameters},
        }
        for c in caps
    ]


def measure(full: list, selected: list, renderer: Callable) -> dict:
    """Before/after rendered size on the same renderer. Tokens are null without a tokenizer."""

    def size(caps: list) -> int:
        rendered = renderer(caps)
        text = rendered if isinstance(rendered, str) else jcs(rendered)
        return len(text.encode("utf-8"))

    return {
        "full_count": len(full),
        "selected_count": len(selected),
        "full_rendered_bytes": size(full),
        "selected_rendered_bytes": size(selected),
        "full_tokens": None,
        "selected_tokens": None,
        "tokenizer": None,
    }


__all__ = [
    "MappingError",
    "MappingTable",
    "Rule",
    "coerce_arguments",
    "select",
    "measure",
    "openai_tools_renderer",
]
