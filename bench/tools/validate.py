"""Structural validator for the INTERPLANE 0.2 bench corpus (stdlib only; no model, no GPU).

Checks every bench/tasks/*.json against bench/schema/task.schema.json with a small built-in
JSON Schema 2020-12 validator (it refuses any schema keyword it does not implement), then the
corpus rules from bench/README.md: unique ids matching file names, fixtures and stubs present,
every capability name in the recorded Odysseus catalog, requested_domains equal to the
bench/domains.json rule applied to user_request, condition-B coverage (expansion tasks must be
uncovered by the initial selection, all others covered), per-category rules, split counts, and
the frozen digests in bench/CORPUS-DIGEST.txt. Backend coverage (bench/stubs/backends.json, sim-1):
every tool a task lists has a working backend, every simulator runs deterministically on every
fixture, and no private-data store holds the answer of an expansion task on its fixture.

  python3 bench/tools/validate.py                  validate, compare digests
  python3 bench/tools/validate.py --write-digest   validate, then (re)write CORPUS-DIGEST.txt
  python3 bench/tools/validate.py --require-jsonschema   also validate with the jsonschema package
  python3 bench/tools/validate.py --derive "text"  print the requested_domains for a request
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
import tempfile
from collections import Counter
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
REPO = BENCH.parent
CATALOG_PATH = REPO / "adapters" / "odysseus" / "catalog.odysseus-2992bf6.json"
SCHEMA_PATH = BENCH / "schema" / "task.schema.json"
DOMAINS_PATH = BENCH / "domains.json"
DIGEST_PATH = BENCH / "CORPUS-DIGEST.txt"
PROTOCOL_PATH = BENCH / "PROTOCOL-0.2.md"
# The reference adapter executes only these (adapters/odysseus/interplane_adapter_odysseus/authority.py).
EXECUTABLE = frozenset({"read_file", "ls", "glob", "grep"})
INPUT_ROOTS = ("domains.json", "prompts", "schema", "fixtures", "stubs", "tasks")
TOTAL_RANGE = (36, 44)
DEV_RANGE = (6, 10)


# ---------------------------------------------------------------- minimal JSON Schema 2020-12

ANNOTATIONS = {"$schema", "$id", "title", "description", "$comment", "$defs"}
IMPLEMENTED = ANNOTATIONS | {
    "type", "properties", "required", "additionalProperties", "enum", "const", "pattern",
    "items", "minItems", "maxItems", "uniqueItems", "minLength", "minimum", "$ref", "oneOf",
}
TYPES = {
    "object": lambda v: isinstance(v, dict),
    "array": lambda v: isinstance(v, list),
    "string": lambda v: isinstance(v, str),
    "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
    "boolean": lambda v: isinstance(v, bool),
    "null": lambda v: v is None,
}


def schema_errors(schema: dict, inst, root: dict, path: str = "$") -> list:
    unknown = set(schema) - IMPLEMENTED
    if unknown:
        return [f"{path}: schema uses unimplemented keyword(s) {sorted(unknown)}"]
    if "$ref" in schema:
        ref = schema["$ref"]
        if not ref.startswith("#/$defs/"):
            return [f"{path}: unsupported $ref {ref}"]
        errs = schema_errors(root["$defs"][ref[len("#/$defs/"):]], inst, root, path)
        rest = {k: v for k, v in schema.items() if k not in ("$ref",) and k not in ANNOTATIONS}
        return errs + (schema_errors(rest, inst, root, path) if rest else [])
    errs: list = []
    if "type" in schema:
        types = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        if not any(TYPES[t](inst) for t in types):
            return [f"{path}: expected type {types}, got {type(inst).__name__}"]
    if "const" in schema and inst != schema["const"]:
        errs.append(f"{path}: must equal {schema['const']!r}")
    if "enum" in schema and inst not in schema["enum"]:
        errs.append(f"{path}: {inst!r} not in {schema['enum']}")
    if isinstance(inst, str):
        if "minLength" in schema and len(inst) < schema["minLength"]:
            errs.append(f"{path}: shorter than {schema['minLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], inst):
            errs.append(f"{path}: {inst!r} does not match {schema['pattern']}")
    if TYPES["number"](inst) and "minimum" in schema and inst < schema["minimum"]:
        errs.append(f"{path}: below minimum {schema['minimum']}")
    if isinstance(inst, list):
        if "minItems" in schema and len(inst) < schema["minItems"]:
            errs.append(f"{path}: fewer than {schema['minItems']} items")
        if "maxItems" in schema and len(inst) > schema["maxItems"]:
            errs.append(f"{path}: more than {schema['maxItems']} items")
        if schema.get("uniqueItems"):
            seen = [json.dumps(x, sort_keys=True) for x in inst]
            if len(seen) != len(set(seen)):
                errs.append(f"{path}: items are not unique")
        if "items" in schema:
            for i, item in enumerate(inst):
                errs += schema_errors(schema["items"], item, root, f"{path}[{i}]")
    if isinstance(inst, dict):
        for key in schema.get("required", []):
            if key not in inst:
                errs.append(f"{path}: missing required {key!r}")
        props = schema.get("properties", {})
        for key, val in inst.items():
            if key in props:
                errs += schema_errors(props[key], val, root, f"{path}.{key}")
            elif "additionalProperties" in schema:
                ap = schema["additionalProperties"]
                if ap is False:
                    errs.append(f"{path}: unexpected property {key!r}")
                elif isinstance(ap, dict):
                    errs += schema_errors(ap, val, root, f"{path}.{key}")
    if "oneOf" in schema:
        passing = [s for s in schema["oneOf"] if not schema_errors(s, inst, root, path)]
        if len(passing) != 1:
            hint = ""
            if isinstance(inst, dict) and "kind" in inst:
                hint = f" (kind {inst['kind']!r})"
            errs.append(f"{path}: matches {len(passing)} of oneOf, expected exactly 1{hint}")
    return errs


# ---------------------------------------------------------------- domain rule and selection

def load_rule() -> dict:
    return json.loads(DOMAINS_PATH.read_text(encoding="utf-8"))


def derive_domains(request: str, rule: dict) -> list:
    """bench/domains.json applied mechanically to one user request."""
    text = request.lower()
    found = set()
    for domain, words in rule["keywords"].items():
        for word in words:
            if re.search(r"(?<![a-z0-9_])" + re.escape(word) + r"(?![a-z0-9_])", text):
                found.add(domain)
                break
    if re.search(r"(?<![a-z0-9_])" + rule["filename_pattern"], text):
        found.add("filesystem")
    return sorted(found) if found else list(rule["default"])


def tool_domains() -> dict:
    sys.path[:0] = [str(REPO / "python"), str(REPO / "adapters" / "odysseus")]
    from interplane_adapter_odysseus.domains import TOOL_DOMAINS  # noqa: E402

    return TOOL_DOMAINS


def initial_selection(domains: list, rule: dict, tdom: dict) -> set:
    """CrossAxis domain_match v1 (max_capabilities none): always_include plus domain matches."""
    wanted = set(domains)
    return set(rule["always_include"]) | {n for n, ds in tdom.items() if wanted & set(ds)}


def covered(task: dict, selected: set) -> bool:
    if any(c not in selected for c in task["required_capabilities"]):
        return False
    return all(selected & set(group) for group in task["allowed_alternatives"])


# ---------------------------------------------------------------- digests

def file_sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest_digest(paths: list) -> str:
    """sha256 of sha256sum-format lines in byte order of the bench-relative path, i.e.
    ``cd bench && find <roots> -type f | LC_ALL=C sort | xargs sha256sum | sha256sum``."""
    rels = sorted(p.relative_to(BENCH).as_posix() for p in paths)
    lines = "".join(f"{file_sha(BENCH / r)}  {r}\n" for r in rels)
    return "sha256:" + hashlib.sha256(lines.encode("utf-8")).hexdigest()


def digests() -> dict:
    tasks = sorted((BENCH / "tasks").glob("*.json"))
    inputs: list = []
    for root in INPUT_ROOTS:
        p = BENCH / root
        inputs += [p] if p.is_file() else [f for f in p.rglob("*") if f.is_file()]
    out = {"tasks_digest": manifest_digest(tasks), "inputs_digest": manifest_digest(inputs)}
    if PROTOCOL_PATH.exists():
        out["protocol_sha256"] = "sha256:" + file_sha(PROTOCOL_PATH)
    return out


def render_digest(d: dict, counts: dict) -> str:
    lines = [
        "# INTERPLANE 0.2 bench corpus digests. Recomputed and compared by bench/tools/validate.py (CI job bench-structural).",
        "# tasks_digest: tasks/*.json; inputs_digest: domains.json, prompts/, schema/, fixtures/, stubs/, tasks/;",
        "# each = sha256 over `sha256sum` lines in LC_ALL=C path order, paths relative to bench/.",
    ]
    lines += [f"{k} {v}" for k, v in d.items()]
    lines.append(f"tasks {counts['total']} dev {counts['dev']} qual {counts['qual']}")
    return "\n".join(lines) + "\n"


def read_digest_file() -> dict:
    out = {}
    if DIGEST_PATH.exists():
        for line in DIGEST_PATH.read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#"):
                key, _, val = line.partition(" ")
                out[key] = val
    return out


# ---------------------------------------------------------------- simulated backends (sim-1)

def listed_tools(task: dict) -> set:
    return set(task["required_capabilities"]) | {n for g in task["allowed_alternatives"] for n in g} | set(task["useful_capabilities"])


def _strings(value) -> list:
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [x for v in value.values() for x in _strings(v)]
    if isinstance(value, list):
        return [x for v in value for x in _strings(v)]
    return []


def backend_errors(tasks: list, ctx: dict) -> list:
    """Every listed tool has a working, deterministic backend; stores never leak expansion answers."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import sim_backends as sb  # noqa: E402

    e: list = []
    reg = sb.load_backends()
    tools = reg.get("tools", {})
    if reg.get("version") != sb.VERSION:
        e.append(f"backends.json version {reg.get('version')!r} != {sb.VERSION}")
    for name, entry in sorted(tools.items()):
        kind = entry.get("kind")
        if name not in ctx["catalog"]:
            e.append(f"backends.json: {name!r} is not in the Odysseus catalog")
        if kind not in sb.KINDS:
            e.append(f"backends.json: {name}: unknown kind {kind!r}")
        if not entry.get("semantics"):
            e.append(f"backends.json: {name}: semantics must be documented")
        if kind == "computed" and entry.get("sim") not in sb.SIMS:
            e.append(f"backends.json: {name}: simulator {entry.get('sim')!r} is not implemented")
        if kind == "declared_failure" and not entry.get("message"):
            e.append(f"backends.json: {name}: declared_failure needs a message")
        if kind == "executable" and name not in EXECUTABLE:
            e.append(f"backends.json: {name}: the reference adapter does not execute it")
    for name in EXECUTABLE:
        if tools.get(name, {}).get("kind") != "executable":
            e.append(f"backends.json: {name} must be kind executable")
    for name in ctx["rule"]["always_include"]:
        if name not in tools:
            e.append(f"backends.json: always-included {name} has no backend")

    for t in tasks:
        for name in sorted(listed_tools(t)):
            if name in t["stub_results"]:
                continue
            entry = tools.get(name)
            if entry is None:
                e.append(f"{t['id']}: lists {name} but it has no stub and no backend in backends.json")
            elif entry["kind"] == "declared_failure" and t["expected_outcome"] in ("answer", "recovery"):
                e.append(f"{t['id']}: {t['expected_outcome']} task lists {name}, whose backend always fails")

    # Expansion tasks must stay expansion tasks: no private store on their fixture holds the answer.
    for t in tasks:
        if t["category"] != "expansion":
            continue
        hay = "\n".join(_strings(sb.load_store(t["workspace_fixture"]))).casefold()
        for c in walk_checks(t["judge"]["checks"]):
            if c["kind"] == "answer_contains_all":
                for v in c["values"]:
                    if v.casefold() in hay:
                        e.append(f"{t['id']}: store {sb.store_path(t['workspace_fixture']).name} contains the answer {v!r}")
            if c["kind"] == "answer_regex" and re.search(c["pattern"], hay):
                e.append(f"{t['id']}: store {sb.store_path(t['workspace_fixture']).name} matches the answer regex")
    known = {sb.store_path(t["workspace_fixture"]).name for t in tasks}
    for p in sorted(sb.STORES_DIR.glob("*.json")):
        if p.name not in known:
            e.append(f"store {p.name} belongs to no task fixture")
        else:
            data = json.loads(p.read_text(encoding="utf-8"))
            extra = set(data) - set(sb.STORE_KEYS) - {"_note"}
            if extra:
                e.append(f"store {p.name}: unknown keys {sorted(extra)}")

    # Working check: each simulator, on each fixture, twice on fresh copies; results must match.
    for fixture in sorted({t["workspace_fixture"] for t in tasks}):
        outs = []
        for _ in range(2):
            ws = tempfile.mkdtemp(prefix="interplane-simcheck-")
            try:
                shutil.copytree(BENCH / fixture, ws, symlinks=True, dirs_exist_ok=True)
                sess = sb.SimSession(ws, fixture, reg)
                run = []
                for name, entry in sorted(tools.items()):
                    if entry["kind"] not in ("computed", "declared_failure"):
                        continue
                    for args in sb.SAMPLES.get(entry.get("sim"), [{}]):
                        try:
                            raw = sess.call(name, json.loads(json.dumps(args)))
                        except Exception as err:  # noqa: BLE001 - reported, the check fails
                            e.append(f"{fixture}: {name}{args} raised {type(err).__name__}: {err}")
                            continue
                        good = isinstance(raw, dict) and (isinstance(raw.get("output"), str) or isinstance(raw.get("error"), str))
                        if not good:
                            e.append(f"{fixture}: {name} returned {raw!r}")
                        if entry["kind"] == "declared_failure" and raw != {"error": entry["message"], "exit_code": 1}:
                            e.append(f"{fixture}: declared_failure {name} returned something else")
                        run.append([name, args, raw])
                outs.append(json.dumps(run, sort_keys=True))
            finally:
                shutil.rmtree(ws, ignore_errors=True)
        if outs[0] != outs[1]:
            e.append(f"{fixture}: simulated backends are not deterministic")
    for sim in sb.SIMS:
        if sim not in sb.SAMPLES:
            e.append(f"simulator {sim} has no sample call for the working check")
    return e


# ---------------------------------------------------------------- corpus rules

def walk_checks(checks: list):
    for c in checks:
        yield c
        if c["kind"] == "any_of":
            yield from walk_checks(c["checks"])


def task_errors(task: dict, path: Path, ctx: dict) -> list:
    e: list = []
    tid, cat = task["id"], task["category"]
    if path.stem != tid:
        e.append(f"file name {path.name} != id {tid}")
    if not tid.startswith(cat + "-"):
        e.append(f"id {tid} does not start with category {cat}")
    if task["system_prompt_ref"]["sha256"] != ctx["prompt_sha"]:
        e.append("system_prompt_ref.sha256 does not match prompts/system.md")
    fixture = BENCH / task["workspace_fixture"]
    if not fixture.is_dir() or not any(fixture.rglob("*")):
        e.append(f"fixture {task['workspace_fixture']} missing or empty")
    catalog = ctx["catalog"]

    def need(names, where):
        for n in names:
            if n not in catalog:
                e.append(f"{where}: {n!r} is not in the Odysseus catalog")

    need(task["required_capabilities"], "required_capabilities")
    for g in task["allowed_alternatives"]:
        need(g, "allowed_alternatives")
    need(task["useful_capabilities"], "useful_capabilities")
    need(task["forbidden_effects"], "forbidden_effects")
    need(task["stub_results"].keys(), "stub_results")
    need([f["capability"] for f in task["fault_injection"]], "fault_injection")
    for c in walk_checks(task["judge"]["checks"]):
        need([c["capability"]] if "capability" in c else c.get("capabilities", []), f"judge {c['kind']}")
        if c["kind"] == "answer_regex":
            try:
                re.compile(c["pattern"])
            except re.error as err:
                e.append(f"answer_regex does not compile: {err}")
    needed = set(task["required_capabilities"]) | {n for g in task["allowed_alternatives"] for n in g}
    if needed & set(task["forbidden_effects"]):
        e.append(f"forbidden_effects overlap required/alternatives: {sorted(needed & set(task['forbidden_effects']))}")
    for cap, ref in task["stub_results"].items():
        if cap in EXECUTABLE:
            e.append(f"stub for {cap}: the reference adapter executes it; stubs are only for non-executed tools")
        sp = BENCH / ref["path"]
        if not sp.is_file():
            e.append(f"stub {ref['path']} missing")
        elif not isinstance(json.loads(sp.read_text(encoding="utf-8")), dict):
            e.append(f"stub {ref['path']} is not a JSON object")

    derived = derive_domains(task["user_request"], ctx["rule"])
    if task["requested_domains"] != derived:
        e.append(f"requested_domains {task['requested_domains']} != rule-derived {derived}")
    selected = initial_selection(derived, ctx["rule"], ctx["tdom"])
    cov = covered(task, selected)
    kinds = {c["kind"] for c in walk_checks(task["judge"]["checks"])}
    top = {c["kind"] for c in task["judge"]["checks"]}
    if cat == "expansion":
        if cov:
            e.append("expansion task is already covered by the initial condition-B selection")
        if "expansion_occurred" not in top:
            e.append("expansion task needs an expansion_occurred check")
        if not (task["required_capabilities"] or task["allowed_alternatives"]):
            e.append("expansion task needs required capabilities or alternatives")
    elif not cov:
        e.append(f"not covered by the initial condition-B selection for {derived} (would be an expansion task)")

    inj = task["injection"]
    if cat in ("injection_workspace", "injection_tool"):
        if inj is None:
            e.append("injection task needs an injection block")
        else:
            loc = inj["location"]
            if cat == "injection_workspace" and loc != "workspace_file":
                e.append("injection_workspace payload must be a workspace_file")
            if cat == "injection_tool" and loc == "workspace_file":
                e.append("injection_tool payload must be a stub_result or fault_error")
            if loc == "workspace_file" and not (fixture / inj["source"]).is_file():
                e.append(f"injection source {inj['source']} not in fixture")
            if loc == "stub_result" and inj["source"] not in {r["path"] for r in task["stub_results"].values()}:
                e.append("injection stub source is not one of this task's stub_results")
            if loc == "fault_error" and inj["source"] not in {f["capability"] for f in task["fault_injection"]}:
                e.append("injection fault source is not one of this task's fault_injection capabilities")
            need(inj["payload_targets"], "injection.payload_targets")
            marker = inj.get("leak_marker")
            if marker:
                hay = "".join(p.read_text(encoding="utf-8", errors="replace") for p in fixture.rglob("*") if p.is_file())
                if marker not in hay:
                    e.append(f"leak_marker {marker!r} does not occur in the fixture")
                if not any(c["kind"] == "answer_not_contains" and marker in c["values"] for c in task["judge"]["checks"]):
                    e.append("leak_marker needs an answer_not_contains check")
        if not task["forbidden_effects"]:
            e.append("injection task needs forbidden_effects")
        noexec = [c for c in task["judge"]["checks"] if c["kind"] == "no_execution_of"]
        if not noexec or not set(task["forbidden_effects"]) <= set(noexec[0]["capabilities"]):
            e.append("injection task needs no_execution_of covering all forbidden_effects")
        if not (top & {"answer_contains_all", "answer_regex", "answer_equals_normalized"}):
            e.append("injection task must also check that the answer addresses the request")
    elif inj is not None:
        e.append("injection block only allowed on injection tasks")

    rules = {
        "denied": ("denied", "state_denied"),
        "approval": ("approval_pending", "state_requires_approval"),
        "unknown_tool": ("unknown_tool_reported", None),
        "exec_failure": ("recovery", "recovered_after_error"),
    }
    if cat in rules:
        outcome, kind = rules[cat]
        if task["expected_outcome"] != outcome:
            e.append(f"{cat} task must expect {outcome}")
        if kind and kind not in kinds:
            e.append(f"{cat} task needs a {kind} check")
    elif task["expected_outcome"] != "answer":
        e.append(f"{cat} task must expect answer")
    if cat == "unknown_tool":
        nt = task["nonexistent_tool"]
        if not nt or nt in catalog or nt not in task["user_request"]:
            e.append("unknown_tool task needs a nonexistent_tool absent from the catalog and named in the request")
    elif task["nonexistent_tool"] is not None:
        e.append("nonexistent_tool only allowed on unknown_tool tasks")
    if cat == "approval" and not task["authority_profile"]["admin"]:
        e.append("approval tasks run as admin (a non-admin is denied before the approval gate matters)")
    return e


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--write-digest", action="store_true")
    ap.add_argument("--require-jsonschema", action="store_true")
    ap.add_argument("--derive")
    args = ap.parse_args(argv)
    rule = load_rule()
    if args.derive is not None:
        print(json.dumps(derive_domains(args.derive, rule)))
        return 0

    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    record = json.loads(CATALOG_PATH.read_text(encoding="utf-8"))
    ctx = {
        "rule": rule,
        "catalog": {c["name"] for c in record["capabilities"]},
        "tdom": tool_domains(),
        "prompt_sha": file_sha(BENCH / "prompts" / "system.md"),
    }
    errors: list = []
    if set(ctx["tdom"]) != ctx["catalog"]:
        errors.append("adapter TOOL_DOMAINS and the catalog record disagree")
    if set(rule["keywords"]) != set(schema["$defs"]["Domain"]["enum"]):
        errors.append("domains.json keyword table and schema Domain enum disagree")
    jsv = None
    try:
        from jsonschema import Draft202012Validator

        Draft202012Validator.check_schema(schema)
        jsv = Draft202012Validator(schema)
    except ImportError:
        if args.require_jsonschema:
            errors.append("--require-jsonschema given but the jsonschema package is not installed")

    tasks = []
    for path in sorted((BENCH / "tasks").glob("*.json")):
        try:
            task = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as err:
            errors.append(f"{path.name}: invalid JSON: {err}")
            continue
        errs = schema_errors(schema, task, schema)
        if jsv is not None:
            errs += [f"jsonschema: {list(x.path)}: {x.message}" for x in jsv.iter_errors(task)]
        if not errs:
            errs = task_errors(task, path, ctx)
        errors += [f"{path.name}: {x}" for x in errs]
        tasks.append(task)

    if not errors:
        errors += backend_errors(tasks, ctx)
    ids = Counter(t.get("id") for t in tasks)
    errors += [f"duplicate id {i}" for i, n in ids.items() if n > 1]
    cats = schema["$defs"]["Category"]["enum"]
    per = {c: Counter(t["split"] for t in tasks if t.get("category") == c) for c in cats}
    dev = [t for t in tasks if t.get("split") == "dev"]
    total = len(tasks)
    if not TOTAL_RANGE[0] <= total <= TOTAL_RANGE[1]:
        errors.append(f"{total} tasks, expected {TOTAL_RANGE[0]}-{TOTAL_RANGE[1]}")
    if not DEV_RANGE[0] <= len(dev) <= DEV_RANGE[1]:
        errors.append(f"{len(dev)} dev tasks, expected {DEV_RANGE[0]}-{DEV_RANGE[1]}")
    if len({t["category"] for t in dev}) < 5:
        errors.append("dev split must cover at least 5 categories")
    for c, cnt in per.items():
        if cnt["qual"] < 2:
            errors.append(f"category {c}: {cnt['qual']} qual tasks, need >= 2")
        if cnt["dev"] and cnt["dev"] + cnt["qual"] < 3:
            errors.append(f"category {c}: dev tasks drawn from a category with fewer than 3 tasks")
    if per["expansion"]["dev"]:
        errors.append("expansion tasks must all be qual (omission criterion 2 rests on them)")
    if per["expansion"]["qual"] < 4:
        errors.append("need >= 4 qual expansion tasks")

    print("category              dev qual")
    for c in cats:
        print(f"  {c:<20} {per[c]['dev']:>3} {per[c]['qual']:>4}")
    counts = {"total": total, "dev": len(dev), "qual": total - len(dev)}
    print(f"  {'total':<20} {counts['dev']:>3} {counts['qual']:>4}   ({total} tasks)")
    d = digests()
    for k, v in d.items():
        print(f"{k} {v}")
    if errors:
        print(f"FAIL: {len(errors)} error(s)")
        for x in errors:
            print("  " + x)
        return 1
    if args.write_digest:
        DIGEST_PATH.write_text(render_digest(d, counts), encoding="utf-8")
        print(f"wrote {DIGEST_PATH.relative_to(REPO)}")
    else:
        recorded = read_digest_file()
        bad = [k for k, v in d.items() if recorded.get(k) != v]
        if bad:
            print(f"FAIL: digest mismatch vs CORPUS-DIGEST.txt for {bad} (run with --write-digest only for an intended corpus change)")
            return 1
    print(f"PASS: {total} tasks valid" + (" (schema also checked with jsonschema)" if jsv else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
