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

The `measure` block is the before/after receipt; see Measure below.

Why this is the first killer feature: both real runtimes dump every tool schema into context
(Odysseus ~60 function schemas; AIEN CLI a prose list), and small local models get buried. Select
is a receipt-producing, deterministic subset. It grants nothing: an excluded capability is simply
not rendered, and an included one still crosses Crossveil.

## Measure (v0.2)

`selection.measure` is additive; the 0.1 fields (`full_count`, `selected_count`,
`full_rendered_bytes`, `selected_rendered_bytes`) keep their meaning. `full_tokens`,
`selected_tokens` and `tokenizer` are deprecated and are no longer written; readers must still
accept them.

```
measure: {
  rendered_bytes: {full, selected},                 // always present
  tokens?: {
    unit,            // bytes | tokens_model_reported | tokens_endpoint_tokenizer | tokens_estimated
    source,          // free text: "usage.prompt_tokens max_tokens=1 differential",
                     //            "llama.cpp /tokenize", "estimator v1 ceil(utf8_bytes/4)"
    base_context,    // prompt size of system + user, no tools
    tool_schema: {full, selected},
    first_turn_prompt: {full, selected},            // system + user + tools
    all_rounds_prompt?: {full, selected},           // bench total over the whole task
    completion?: {full, selected}                   // bench total
  }
}
```

**Bytes are wire bytes.** `rendered_bytes` is the UTF-8 length of the canonical JSON (RFC 8785) of
the `tools` array produced by the renderer, which must be the renderer whose output is sent
unchanged (`openai_tools_renderer` for OpenAI-style endpoints: `{"type":"function","function":
{name, description, parameters}}`). A sender that serializes with another JSON writer must say so
in `source`; byte counts then are not comparable.

**Unit discipline.** Every number inside one `tokens` block is in the block's single `unit`. A
block never mixes units; a bench that has two kinds of counts writes two runs, not one block.
`bytes` is the unit of `BytesOnly` (canonical-JSON bytes of messages plus tools); it exists so a
no-tokenizer run still has the same shape. `tokens_estimated` is always an estimate and is labelled
so by its unit; it must not be compared with measured units.

**Differential rule.** `first_turn_prompt = count(system+user+tools)`, `base_context =
count(system+user)`, `tool_schema = first_turn_prompt - base_context` (clamped at 0), for
`full` and `selected` alike. Under `tokens_model_reported` the two counts are
`usage.prompt_tokens` of two requests with `max_tokens=1` to the same endpoint with the same
model, system and user messages; the second adds the rendered tools and nothing else. Under
`tokens_endpoint_tokenizer` they are the tokenizer's counts of the same two request bodies.

**TokenCounter.** Core is stdlib-only and offline. It defines the interface and two offline
counters; live counters (model-reported, endpoint tokenizer) are implemented by the bench runner
against this interface and Core only assembles the receipt.

- Python: `TokenCounter` protocol with `unit`, `source`, `count_prompt(messages, tools_or_None) -> int`.
  Rust: `trait TokenCounter { unit(), source(), count_prompt(&Value, Option<&Value>) -> u64 }`.
- `BytesOnly`: `len(jcs(messages)) + len(jcs(tools))` in UTF-8 bytes (tools term absent for `None`).
- `EstimatedTokens` (estimator v1): for a JSON value `v`, `est(v) = ceil(len(utf8(jcs(v))) / 4)`;
  `count_prompt = est(messages) + est(tools)` (second term absent for `None`).
  `source = "estimator v1 ceil(utf8_bytes/4)"`.
- `all_rounds_prompt` and `completion` are totals the bench measured in the block's unit; Core
  validates shape and non-negativity only.

## Bounded expansion (v0.2)

Expansion is an evidence-based, bounded widening of a selection on top of `domain_match` v1. It
is not a semantic router: Core never guesses what the model "meant". It only reacts to evidence.
**Expansion never touches authority.** It changes which descriptors Lenshift renders and nothing
else: every call, expanded-in or not, still crosses Crossveil, where the runtime decides. A
capability added by expansion that the runtime denies is `denied` exactly as before (conformance
case 24).

`expand(selection, catalog, evidence, max_expansions?, max_added_per_expansion?) -> selection'`
is pure and deterministic: the same catalog, selection and evidence give byte-identical
`selection'`. The input is not modified. If `selection.catalog_digest` is not the catalog's digest
the call fails (nothing is recorded).

Evidence (a JSON object; `evidence_digest` is its JCS digest):

1. `{"kind":"requested_excluded","name":N,"include_domain_siblings"?:bool}`. The model called a
   capability that exists in the full catalog but was excluded from the current selection
   (observable as `unknown_capability` at the selected view). Candidates: `N`, then, only when
   `include_domain_siblings` is true, every other capability sharing a domain with `N`, sorted by
   name.
2. `{"kind":"discovery_hit","query"?:Q,"names":[..]}`. `names` is the output of `discover`.
   Candidates: `names` in the given order, duplicates dropped.

`discover(catalog, selection, query, limit=8) -> [name]` is pure: ASCII case-insensitive
substring match of `query` against the name or description of EXCLUDED capabilities (excluded
with reason `runtime_disabled` are never offered), sorted by name, at most `limit`; an empty query
gives `[]`. A runtime or bench may expose `discover` to the model as an always-included read-only
tool; Core defines no such tool in any catalog.

Bounds: `max_expansions` (default 2) and `max_added_per_expansion` (default 8), always within the
selector's `max_capabilities`. The effective bounds are recorded in `selector.expansion`
`{max_expansions, max_added_per_expansion}`; once recorded they win over later arguments, so the
receipt alone determines the result.

Each candidate is checked in this order and the first failing check is its refusal reason:
`not_in_catalog`, `already_selected`, `runtime_disabled`, `max_expansions` (rounds that added
something already equal `max_expansions`), `max_added_per_expansion`, `max_capabilities`
(selected plus added so far already equal `max_capabilities`). Pinned (`always_include`) entries
may already exceed `max_capabilities`; expansion then adds nothing.

Every call appends `{round, reason, evidence_digest, added[], refused[{name, reason}]}` to
`selection.expansions[]`, including a call that adds nothing: `round` is the 1-based position in
that list (refused rounds are numbered but do not count toward `max_expansions`). `reason` is the
evidence kind. Added entries are appended to `selected` (rule id `expansion:<round>`, in candidate
order) and removed from `excluded`; the stale `measure` is dropped (the caller measures again).
`selection_digest` is the JCS digest of the selection with `selection_digest` and `measure`
removed; `expand` sets `parent_digest` to that digest of its input and `selection_digest` on its
output. `select` never writes a digest.

Receipt normalization (both languages): optional fields are omitted when empty (`always_include`,
`selected[].domains`, `selector.max_capabilities` when null).

Reference API. Python (`interplane.crossaxis`): `measure(full, selected, renderer, counter=None,
messages=None, all_rounds_prompt=None, completion=None)`, `token_block(...)`,
`BytesOnly()`, `EstimatedTokens()`, `discover(catalog, selection, query, limit=8)`,
`expand(selection, catalog, evidence, max_expansions=None, max_added_per_expansion=None)`,
`selection_digest(selection)`. Rust (`interplane_crossaxis`): `measure`, `measure_tokens`,
`openai_tools_renderer`, `discover`, `expand`, `selection_digest`.
