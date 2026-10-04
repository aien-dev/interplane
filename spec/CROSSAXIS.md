# CrossAxis: capability coordinates and selection

CrossAxis answers "which runtime capability is this canonical tool?" and "which capabilities
should this model see for this request?". It never answers "may the model run it?".

## Mapping (v0.1, minimal)

A `MappingTable { runtime, table_version, rules[] }` with rules of two kinds:

- `alias`: canonical `{namespace, name}` -> runtime capability name (+ optional argument renames).
- `passthrough`: bare native name -> itself, only for names present in the runtime catalog.

Lookup is exact and deterministic. No fuzzy matching, no "dangerous guess": an unmapped name is
`unknown_capability` and never reaches the runtime. Tables are data (JSON), versioned, inspectable
and tested; the two real adapters each ship one.

Argument renames are explicit per rule (`{"from": "path", "to": "file_path"}`). Type coercion
against the capability's parameter schema (strings from the qwen35 dialect into integers/booleans
the schema declares) happens here, using the catalog's `parameters`, and is recorded as
`mapping.coerced = [keys]`.

## CrossAxis Select (v0.1)

Input: a runtime `catalog`, a list of requested domains, a `max_capabilities`, an `always_include`
list. Output: a `selection` receipt and the selected `CapabilityDescriptor[]` for Lenshift to render.

Selector `domain_match` version 1: capabilities named in `always_include` are selected first (rule id
`always_include`, in catalog order, never truncated); then capabilities with any domain in the
requested set are selected sorted by name (rule id `domain:<matched domain>`); capabilities with no
domains are excluded with `no_domains`, the rest with `domain_mismatch`; when the total exceeds
`max_capabilities`, domain-selected entries are dropped from the end (sorted by name) with reason
`max_capabilities`. Deterministic by construction.

Domain resolution (what the user asked -> requested domains) is **not** in v0.1 Core; the reference
runner takes domains as input. Selecting domains with a classifier or the model itself is a runtime
or bench concern; the receipt records whatever was requested.

The `measure` block is filled by rendering the full catalog and the selection with the same
Lenshift renderer and counting bytes (and tokens when a tokenizer is available). That is the
before/after receipt.

Why this is the first killer feature: both real runtimes dump every tool schema into context
(Odysseus ~60 function schemas; AIEN CLI a prose list), and small local models get buried. Select
is a receipt-producing, deterministic subset. It grants nothing: an excluded capability is simply
not rendered, and an included one still crosses Crossveil.
