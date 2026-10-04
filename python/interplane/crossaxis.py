"""CrossAxis: explicit capability mapping and deterministic domain selection.

CrossAxis answers "which runtime capability is this canonical tool?" and "which capabilities
should this model see?". It never answers "may the model run it?".
"""

import copy
import re
from dataclasses import dataclass, field
from typing import Any, Callable, Optional, Protocol

from .core import (
    DIGEST_RE,
    digest,
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


def _selector(max_capabilities: Optional[int]) -> dict:
    out: dict = {"name": "domain_match", "version": "1"}
    if max_capabilities is not None:
        out["max_capabilities"] = max_capabilities
    return out


def _selected_entry(cap, rule: str) -> dict:
    """Receipt entry; ``domains`` is omitted when empty (same rule as the Rust implementation)."""
    entry = {"name": cap.name, "rule_id": rule}
    if cap.domains:
        entry["domains"] = list(cap.domains)
    return entry


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
        selector=_selector(max_capabilities),
        requested_domains=list(requested_domains),
        always_include=list(always_include) if always_include else None,
        selected=[_selected_entry(c, rule) for c, rule in chosen],
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




# ---------------------------------------------------------------------------------------------
# Measurement: rendered bytes and token accounting (spec/CROSSAXIS.md, "Measure")
# ---------------------------------------------------------------------------------------------

TOKEN_UNITS = ("bytes", "tokens_model_reported", "tokens_endpoint_tokenizer", "tokens_estimated")
ESTIMATOR_SOURCE = "estimator v1 ceil(utf8_bytes/4)"


def _wire_bytes(value: Any) -> int:
    """Bytes of ``value`` serialized as sent on the wire: canonical JSON (RFC 8785), UTF-8."""
    text = value if isinstance(value, str) else jcs(value)
    return len(text.encode("utf-8"))


class TokenCounter(Protocol):
    """Counts one prompt. ``unit`` is one of ``TOKEN_UNITS``; ``source`` is free text for the receipt.

    ``count_prompt(messages, tools)`` returns the size of ``messages`` plus the rendered ``tools``
    list (``None`` means no tools in the request). A live counter (model-reported
    ``usage.prompt_tokens`` with max_tokens=1, an endpoint ``/tokenize``) lives in the bench runner
    and implements this protocol; Core stays offline.
    """

    unit: str
    source: str

    def count_prompt(self, messages: list, tools: Optional[list]) -> int: ...


class BytesOnly:
    """Unit ``bytes``: canonical-JSON bytes of ``messages`` plus, when present, of ``tools``."""

    unit = "bytes"
    source = "canonical json bytes of messages and tools"

    def count_prompt(self, messages: list, tools: Optional[list]) -> int:
        return _wire_bytes(messages) + (_wire_bytes(tools) if tools is not None else 0)


class EstimatedTokens:
    """Unit ``tokens_estimated``. Estimator v1: ``ceil(utf8_len(jcs(value)) / 4)`` per value.

    A prompt is the estimate for ``messages`` plus, when present, the estimate for ``tools``.
    Always labelled as an estimate; never mix it with measured counts.
    """

    unit = "tokens_estimated"
    source = ESTIMATOR_SOURCE

    @staticmethod
    def _est(value: Any) -> int:
        return -(-_wire_bytes(value) // 4)

    def count_prompt(self, messages: list, tools: Optional[list]) -> int:
        return self._est(messages) + (self._est(tools) if tools is not None else 0)


def _pair(value: Any, label: str) -> dict:
    if not (
        isinstance(value, dict)
        and set(value) == {"full", "selected"}
        and all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in value.values())
    ):
        raise ValueError(f"{label} must be {{full, selected}} non-negative integers")
    return {"full": value["full"], "selected": value["selected"]}


def token_block(
    unit: str,
    source: str,
    base_context: int,
    tool_schema: dict,
    first_turn_prompt: dict,
    all_rounds_prompt: Optional[dict] = None,
    completion: Optional[dict] = None,
) -> dict:
    """Assemble ``measure.tokens`` with unit discipline: one unit for every number in the block."""
    if unit not in TOKEN_UNITS:
        raise ValueError(f"unit must be one of {TOKEN_UNITS}")
    if not isinstance(source, str) or not source:
        raise ValueError("source is required")
    if not isinstance(base_context, int) or isinstance(base_context, bool) or base_context < 0:
        raise ValueError("base_context must be a non-negative integer")
    block = {
        "unit": unit,
        "source": source,
        "base_context": base_context,
        "tool_schema": _pair(tool_schema, "tool_schema"),
        "first_turn_prompt": _pair(first_turn_prompt, "first_turn_prompt"),
    }
    if all_rounds_prompt is not None:
        block["all_rounds_prompt"] = _pair(all_rounds_prompt, "all_rounds_prompt")
    if completion is not None:
        block["completion"] = _pair(completion, "completion")
    return block


def measure(
    full: list,
    selected: list,
    renderer: Callable,
    counter: Optional[TokenCounter] = None,
    messages: Optional[list] = None,
    all_rounds_prompt: Optional[dict] = None,
    completion: Optional[dict] = None,
) -> dict:
    """Before/after size on the same renderer; the bytes are those of the ``tools`` array as sent.

    Without ``counter`` the block has counts and ``rendered_bytes`` only. With a counter and the
    base ``messages`` (system + user, as sent), ``tokens`` is filled by prompt differences:
    ``first_turn_prompt = count(messages, tools)``, ``base_context = count(messages, None)``,
    ``tool_schema = first_turn_prompt - base_context`` (clamped at 0). ``all_rounds_prompt`` and ``completion``
    are totals the bench measured in the same unit; they are passed through.
    """
    full_r, sel_r = renderer(full), renderer(selected)
    f_bytes, s_bytes = _wire_bytes(full_r), _wire_bytes(sel_r)
    out: dict = {
        "full_count": len(full),
        "selected_count": len(selected),
        "full_rendered_bytes": f_bytes,
        "selected_rendered_bytes": s_bytes,
        "rendered_bytes": {"full": f_bytes, "selected": s_bytes},
    }
    if counter is not None:
        if messages is None:
            raise ValueError("a counter needs the base messages")
        base = counter.count_prompt(messages, None)
        first_f = counter.count_prompt(messages, full_r)
        first_s = counter.count_prompt(messages, sel_r)
        out["tokens"] = token_block(
            counter.unit,
            counter.source,
            base,
            {"full": max(first_f - base, 0), "selected": max(first_s - base, 0)},
            {"full": first_f, "selected": first_s},
            all_rounds_prompt,
            completion,
        )
    return out


# ---------------------------------------------------------------------------------------------
# Expansion (spec/CROSSAXIS.md, "Bounded expansion"). Changes what is rendered, never authority.
# ---------------------------------------------------------------------------------------------

DEFAULT_MAX_EXPANSIONS = 2
DEFAULT_MAX_ADDED_PER_EXPANSION = 8
DEFAULT_DISCOVER_LIMIT = 8


class ExpansionError(Exception):
    """The evidence or selection is unusable (refusals are recorded in the receipt, not raised)."""


def selection_digest(selection: Any) -> str:
    """Digest of the selection with ``selection_digest`` and ``measure`` removed."""
    d = selection.to_dict() if hasattr(selection, "to_dict") else copy.deepcopy(selection)
    d.pop("selection_digest", None)
    d.pop("measure", None)
    return digest(d)


def _ascii_lower(text: str) -> str:
    return "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in text)


def discover(
    catalog: Catalog, selection: Selection, query: str, limit: int = DEFAULT_DISCOVER_LIMIT
) -> list:
    """Names of EXCLUDED capabilities whose name or description contains ``query``.

    ASCII case-insensitive substring; sorted by name; at most ``limit``. ``runtime_disabled``
    exclusions are never offered. An empty query returns ``[]``. Pure: no state, no I/O.
    """
    q = _ascii_lower(query)
    if not q:
        return []
    off = {e["name"] for e in selection.excluded if e.get("reason") != "runtime_disabled"}
    hits = [
        c.name
        for c in catalog.capabilities
        if c.name in off and (q in _ascii_lower(c.name) or q in _ascii_lower(c.description or ""))
    ]
    return sorted(hits)[: max(limit, 0)]


def expand(
    selection: Selection,
    catalog: Catalog,
    evidence: dict,
    max_expansions: Optional[int] = None,
    max_added_per_expansion: Optional[int] = None,
) -> Selection:
    """Bounded evidence-based expansion. Returns a new Selection; the input is not modified.

    ``evidence`` is ``{"kind": "requested_excluded", "name": N, "include_domain_siblings"?: bool}``
    or ``{"kind": "discovery_hit", "query"?: Q, "names": [..]}``. Bounds recorded in
    ``selector.expansion`` win over the arguments; the arguments win over the defaults (2 and 8);
    the effective bounds are recorded. Refusal reasons and their order: spec/CROSSAXIS.md.
    """
    if not isinstance(evidence, dict):
        raise ExpansionError("evidence must be an object")
    if selection.catalog_digest != (catalog.catalog_digest or catalog.computed_digest()):
        raise ExpansionError("selection was made against a different catalog")
    kind = evidence.get("kind")
    by_name = {c.name: c for c in catalog.capabilities}
    if kind == "requested_excluded":
        name = evidence.get("name")
        if not isinstance(name, str) or not name:
            raise ExpansionError("requested_excluded needs a name")
        candidates = [name]
        if evidence.get("include_domain_siblings") is True and name in by_name:
            doms = set(by_name[name].domains or [])
            candidates += sorted(
                c.name
                for c in catalog.capabilities
                if c.name != name and doms.intersection(c.domains or [])
            )
    elif kind == "discovery_hit":
        names = evidence.get("names")
        if not isinstance(names, list) or not all(isinstance(n, str) for n in names):
            raise ExpansionError("discovery_hit needs names: [string]")
        candidates = names
    else:
        raise ExpansionError(f"unknown evidence kind: {kind!r}")

    out = Selection.from_dict(selection.to_dict())
    parent = selection_digest(selection)
    recorded = out.selector.get("expansion") or {}
    max_exp = recorded.get("max_expansions", max_expansions)
    max_add = recorded.get("max_added_per_expansion", max_added_per_expansion)
    max_exp = DEFAULT_MAX_EXPANSIONS if max_exp is None else max_exp
    max_add = DEFAULT_MAX_ADDED_PER_EXPANSION if max_add is None else max_add
    out.selector["expansion"] = {"max_expansions": max_exp, "max_added_per_expansion": max_add}

    log = list(out.extra.get("expansions") or [])
    used = sum(1 for e in log if e.get("added"))
    round_no = len(log) + 1
    selected_names = {e["name"] for e in out.selected}
    excluded_reason = {e["name"]: e.get("reason") for e in out.excluded}
    cap = out.selector.get("max_capabilities")
    added: list = []
    refused: list = []
    seen: set = set()
    for name in candidates:
        if name in seen:
            continue
        seen.add(name)
        reason = None
        if name not in by_name:
            reason = "not_in_catalog"
        elif name in selected_names:
            reason = "already_selected"
        elif excluded_reason.get(name) == "runtime_disabled":
            reason = "runtime_disabled"
        elif used >= max_exp:
            reason = "max_expansions"
        elif len(added) >= max_add:
            reason = "max_added_per_expansion"
        elif cap is not None and len(selected_names) + len(added) >= cap:
            reason = "max_capabilities"
        if reason is None:
            added.append(name)
        else:
            refused.append({"name": name, "reason": reason})
    for name in added:
        out.selected.append(_selected_entry(by_name[name], f"expansion:{round_no}"))
    out.excluded = [e for e in out.excluded if e["name"] not in added]
    log.append(
        {
            "round": round_no,
            "reason": kind,
            "evidence_digest": digest(evidence),
            "added": added,
            "refused": refused,
        }
    )
    out.extra["expansions"] = log
    out.measure = None
    out.extra["parent_digest"] = parent
    out.extra.pop("selection_digest", None)
    out.extra["selection_digest"] = selection_digest(out)
    return out


__all__ = [
    "BytesOnly",
    "EstimatedTokens",
    "ExpansionError",
    "MappingError",
    "MappingTable",
    "Rule",
    "TOKEN_UNITS",
    "TokenCounter",
    "coerce_arguments",
    "discover",
    "expand",
    "measure",
    "openai_tools_renderer",
    "select",
    "selection_digest",
    "token_block",
]
