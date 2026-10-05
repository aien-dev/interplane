#!/usr/bin/env python3
"""Validate INTERPLANE conformance and dialect fixtures against spec/schemas.

Exit status is non-zero on any failure. Checks:
  * every explicit envelope step validates against envelope.schema.json
  * every intent in dialects/fixtures validates against intent.schema.json
  * fixture shape, 30 required cases, filename == case slug
  * lifecycle/ fixtures: required set, shape, minting decisions valid against decision.schema.json
  * expected.runtime counts agree with the observed records
  * every digest fixture recomputes; InputRecord examples (one per source class) validate
  * negative control: a mutated envelope MUST fail validation
"""
import copy, glob, hashlib, json, os, sys
from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource
from referencing.jsonschema import DRAFT202012

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SCHEMAS = os.path.join(ROOT, "spec", "schemas")
errors = []
def fail(msg): errors.append(msg)

def load(p):
    with open(p, encoding="utf-8") as f: return json.load(f)

registry = Registry()
schemas = {}
for p in sorted(glob.glob(os.path.join(SCHEMAS, "*.json"))):
    s = load(p)
    schemas[os.path.basename(p)] = s
    registry = registry.with_resource(s["$id"], Resource.from_contents(s, default_specification=DRAFT202012))
def validator(name):
    Draft202012Validator.check_schema(schemas[name])
    return Draft202012Validator(schemas[name], registry=registry, format_checker=FormatChecker())
V_ENV, V_INTENT = validator("envelope.schema.json"), validator("intent.schema.json")
V_SEL = validator("selection.schema.json")
V_INPUT = validator("input.schema.json")
V_DEC = validator("decision.schema.json")

def check(v, inst, where):
    errs = sorted(v.iter_errors(inst), key=lambda e: list(e.absolute_path))
    for e in errs[:3]:
        fail("%s: %s at /%s" % (where, e.message[:200], "/".join(map(str, e.absolute_path))))
    return not errs

# ---- JCS (RFC 8785) minimal, for the digest fixture
def _num(x):
    if isinstance(x, int): return str(x)
    if x == int(x) and abs(x) < 1e21: return str(int(x))
    if x == 1e21: return "1e+21"
    r = repr(x)
    if "e" in r: raise ValueError("unsupported float " + r)
    return r
def _esc(s):
    m = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\t": "\\t", "\n": "\\n", "\f": "\\f", "\r": "\\r"}
    return '"' + "".join(m.get(c, ("\\u%04x" % ord(c)) if ord(c) < 0x20 else c) for c in s) + '"'
def jcs(x):
    if x is None: return "null"
    if x is True: return "true"
    if x is False: return "false"
    if isinstance(x, (int, float)): return _num(x)
    if isinstance(x, str): return _esc(x)
    if isinstance(x, list): return "[" + ",".join(jcs(i) for i in x) + "]"
    return "{" + ",".join(_esc(k) + ":" + jcs(x[k]) for k in sorted(x, key=lambda k: k.encode("utf-16-be"))) + "}"

# ---- conformance fixtures
FIX = os.path.join(ROOT, "conformance", "fixtures")
REQUIRED = {
 "01-valid-tool-request","02-malformed-arguments","03-nonexistent-tool","04-unknown-capability",
 "05-policy-denial","06-explicit-approval","07-successful-execution","08-execution-error","09-timeout",
 "10-multiple-sequential-calls","11-model-continuation","12-unsupported-dialect","13-unsupported-version",
 "14-unknown-fields","15-oversized-arguments","16-duplicate-request-ids","17-replay","18-model-retry-after-denial",
 "19-valid-write-request","20-missing-required-argument","21-stale-capability-mapping","22-untrusted-tool-result","23-untrusted-memory-result",
 "24-expansion-requested-excluded","25-expansion-refused-by-bound",
 "26-unrecognized-content-kind","27-absent-null-unrecognized-trust",
 "28-input-ledger-continues","29-input-derived-from","30-input-forged-exposure",
 "31-source-class-workspace","32-source-class-web","33-source-class-memory","34-source-class-document","35-source-class-skill","36-source-class-tool-output","37-source-class-external-provider","38-source-class-runtime-generated","39-source-class-user-request","40-source-class-model-generated"}
OBS_KEYS = ["request_id","stage","decision","status","error_code","decide_invoked","execute_invoked","result_digest"]
n_env = n_cases = 0
neg_done = False
seen = set()
for p in sorted(glob.glob(os.path.join(FIX, "*.json"))):
    f = load(p); name = os.path.basename(p)[:-5]; n_cases += 1; seen.add(name)
    if f.get("case") != name: fail("%s: case slug %r != filename" % (name, f.get("case")))
    for k in ("description","limits","trace_id","steps","expected"):
        if k not in f: fail("%s: missing %s" % (name, k))
    exp = f.get("expected", {})
    obs = exp.get("observed", [])
    for i, o in enumerate(obs):
        if list(o.keys()) != OBS_KEYS: fail("%s: observed[%d] keys/order %s" % (name, i, list(o.keys())))
        if o.get("result_digest") is not None: fail("%s: observed[%d] result_digest must be null" % (name, i))
    rt = exp.get("runtime", {})
    if rt.get("decide_calls") != sum(1 for o in obs if o["decide_invoked"]): fail("%s: decide_calls mismatch" % name)
    if rt.get("execute_calls") != sum(1 for o in obs if o["execute_invoked"]): fail("%s: execute_calls mismatch" % name)
    for s in f.get("steps", []):
        if "envelope" in s:
            n_env += 1
            if check(V_ENV, s["envelope"], name + " step %s envelope" % s.get("turn")) and not neg_done:
                bad = copy.deepcopy(s["envelope"]); bad["payload"]["tool"]["unexpected"] = 1
                if V_ENV.is_valid(bad): fail("negative control: mutated envelope (extra key in tool) unexpectedly valid")
                bad = copy.deepcopy(s["envelope"]); del bad["trace_id"]
                if V_ENV.is_valid(bad): fail("negative control: envelope without trace_id unexpectedly valid")
                neg_done = True
        elif "dialect" in s and "input" in s: pass
        elif "input" in s and "dialect" not in s:  # host-only input registration (0.3 cut P3)
            check(V_INPUT, s["input"], name + " input step")
        elif "expand" in s:
            if "selection" not in f: fail("%s: expand step without a selection block" % name)
            ev = s["expand"].get("evidence", {})
            if ev.get("kind") not in ("requested_excluded", "discovery_hit"): fail("%s: bad evidence kind" % name)
        else: fail("%s: step %s has neither envelope nor dialect+input" % (name, s.get("turn")))
    for rec in exp.get("inputs", []):
        check(V_INPUT, rec, name + " expected.inputs") if "content_digest" in rec else None
    for e in exp.get("exposure", []):
        if set(e) != {"request_id", "inputs", "floor"}: fail("%s: malformed expected.exposure %s" % (name, e))
    for c in exp.get("result_checks", []):
        if set(c) - {"round_trip"} != {"request_id", "path", "equals"} or c.get("round_trip", True) is not True or c["request_id"] not in {o["request_id"] for o in obs}:
            fail("%s: malformed result_check %s" % (name, c))
    if ("selection" in f) != (name[:2] in ("24", "25")): fail("%s: selection block only in cases 24-25" % name)
    if "selection" in f:
        n_exp = sum(1 for s in f["steps"] if "expand" in s)
        sels = exp.get("selections", [])
        if len(sels) != n_exp: fail("%s: expected.selections must have one entry per expand step" % name)
        for i, sel in enumerate(sels):
            check(V_SEL, sel, "%s selections[%d]" % (name, i))
            if sel.get("selection_digest") != "sha256:" + hashlib.sha256(jcs({k: v for k, v in sel.items() if k not in ("selection_digest", "measure")}).encode()).hexdigest():
                fail("%s: selections[%d] selection_digest does not recompute" % (name, i))
            if i and sel.get("parent_digest") != sels[i - 1].get("selection_digest"): fail("%s: parent_digest chain broken at %d" % (name, i))
    if ("mapping_table" in f) != name.startswith("21"): fail("%s: mapping_table present only in case 21" % name)
    if ("mock_provenance" in f) != (name[:2] in ("26", "27")): fail("%s: mock_provenance present only in cases 26-27" % name)
    for cap, o in f.get("mock_provenance", {}).items():
        if not set(o) <= {"content_kind", "trust", "trusted"}: fail("%s: mock_provenance.%s keys %s" % (name, cap, sorted(o)))
    if name[:2] in ("26", "27"):
        rt_paths = {(c["request_id"], c["path"]) for c in exp.get("result_checks", []) if c.get("round_trip")}
        plain = {(c["request_id"], c["path"]) for c in exp.get("result_checks", []) if not c.get("round_trip")}
        if not plain or rt_paths != plain: fail("%s: every provenance check must also be checked after a JCS round trip" % name)
        for c in exp.get("result_checks", []):
            if c["path"] == "provenance.trusted" and c["equals"] is True: fail("%s: trusted must never be true here" % name)
    if name.startswith("21") and f.get("mapping_table") != "mock-table-stale": fail("21: mapping_table must be mock-table-stale")
    if name[:2] in ("19", "22", "23", "20", "21"):
        want = {"19": ("SUCCEEDED", "ok"), "20": ("REJECTED", "rejected"), "21": ("REJECTED", "rejected"),
                "22": ("SUCCEEDED", "ok"), "23": ("SUCCEEDED", "ok")}[name[:2]]
        if (obs[0]["stage"], obs[0]["status"]) != want: fail("%s: unexpected stage/status %s" % (name, (obs[0]["stage"], obs[0]["status"])))
        pairs = {(c["path"], json.dumps(c["equals"])) for c in exp.get("result_checks", [])}
        need = {"19": [("data.appended", "10")],
                "20": [("error.message", '"missing required argument: path"')],
                "21": [("error.message", '"mapping table catalog digest does not match runtime catalog"')],
                "22": [("provenance.trust", '"external_untrusted"'), ("provenance.content_kind", '"web_content"')],
                "23": [("provenance.trust", '"workspace_untrusted"'), ("provenance.content_kind", '"memory"')]}[name[:2]]
        for n in need:
            if n not in pairs: fail("%s: missing result_check %s" % (name, n))
    if name.startswith("22"):
        if not f["steps"][1].get("continues") or [t["outcome"] for t in exp["turns"]] != ["tool_request", "no_tool"]: fail("22: continuation shape")
        if len(obs) != 1: fail("22: the continuing turn must add no records")
        if "<tool_call>" not in next(c["equals"] for c in exp["result_checks"] if c["path"] == "data.content"): fail("22: pinned content must carry the markup")
    # size checks for the oversized case
    if name.startswith("15"):
        a = json.loads(f["steps"][0]["input"]["tool_calls"][0]["function"]["arguments"])
        if len(a["content"]) != 70000 or set(a["content"]) != {"a"}: fail("15: content must be 70000 x 'a'")
        if len(jcs(a).encode()) <= 65536: fail("15: arguments not oversized")
if seen != REQUIRED: fail("required cases missing/extra: %s" % sorted(seen ^ REQUIRED))
if not neg_done: fail("negative control never ran")

# ---- lifecycle fixtures (driven against the Lifecycle directly, no pipeline)
LIFECYCLE = {"01-forged-approval-id", "02-empty-approval-id", "03-approval-id-from-other-request",
             "04-runtime-minted-approval-id", "05-continuation-non-authorized-value"}
STATES = {"PROPOSED","REJECTED","MAPPED","DENIED","REQUIRES_APPROVAL","AUTHORIZED","EXECUTING","SUCCEEDED","FAILED","TIMED_OUT"}
n_life = 0; lseen = set()
for p in sorted(glob.glob(os.path.join(FIX, "lifecycle", "*.json"))):
    f = load(p); name = os.path.basename(p)[:-5]; n_life += 1; lseen.add(name)
    if f.get("case") != "lifecycle/" + name: fail("lifecycle/%s: case slug %r != filename" % (name, f.get("case")))
    if set(f) != {"case", "description", "steps", "expected"}: fail("lifecycle/%s: keys %s" % (name, sorted(f)))
    steps, exp = f.get("steps", []), f.get("expected", [])
    if len(steps) != len(exp): fail("lifecycle/%s: one expected row per step" % name)
    minted = set()
    for i, (s, e) in enumerate(zip(steps, exp)):
        d = s.get("decision", {})
        if list(e) != ["request_id", "state", "refused"] or e["state"] not in STATES or e["request_id"] != s.get("request_id"):
            fail("lifecycle/%s: expected[%d] shape" % (name, i))
        if s.get("request_id") not in minted:
            # first decision on a request is the runtime's own: requires_approval with a minted id
            check(V_DEC, d, "lifecycle/%s step %d minting decision" % (name, i))
            if d.get("decision") != "requires_approval" or d.get("request_id") != s.get("request_id"): fail("lifecycle/%s: step %d must mint" % (name, i))
            minted.add(s.get("request_id"))
        elif d.get("request_id") != s.get("request_id"): fail("lifecycle/%s: step %d continuation must cite its own request_id" % (name, i))
if lseen != LIFECYCLE: fail("lifecycle cases missing/extra: %s" % sorted(lseen ^ LIFECYCLE))

# ---- digest fixtures (every digest/*.json; a "type" is a schema-checked InputRecord or Exposure)
n_dig = 0
for dp in sorted(glob.glob(os.path.join(FIX, "digest", "*.json"))):
    d = load(dp); dn = "digest/" + os.path.basename(dp)[:-5]; n_dig += 1
    got = jcs(d["value"])
    if got != d["expected_jcs"]: fail("%s: expected_jcs mismatch" % dn)
    if "sha256:" + hashlib.sha256(got.encode("utf-8")).hexdigest() != d["expected_sha256"]: fail("%s: sha256 mismatch" % dn)
    if d.get("type") == "InputRecord": check(V_INPUT, d["value"], dn)
    elif d.get("type") == "Exposure":
        check(V_INTENT, {"kind": "tool_request", "request_id": "r", "tool": {"name": "t"}, "arguments": {},
                         "provenance": {"dialect": "d", "parser_version": "0.1.0", "exposure": d["value"]}}, dn)
    elif "type" in d: fail("%s: unknown type %r" % (dn, d["type"]))

# ---- InputRecord examples: one per source class (plan cut P2, protocol gate P3)
INPUT_CLASSES = {"workspace": "workspace_content", "web": "web_content", "memory": "memory", "document": "document",
                 "skill": "skill", "tool-output": "tool_result", "external-provider": "external_provider",
                 "runtime-generated": "runtime_instruction", "user-request": "user_request"}
have_in = {os.path.basename(x)[:-5] for x in glob.glob(os.path.join(FIX, "input", "*.json"))}
if have_in != set(INPUT_CLASSES): fail("input examples differ: %s" % sorted(have_in ^ set(INPUT_CLASSES)))
for cls, kind in INPUT_CLASSES.items():
    ip = os.path.join(FIX, "input", cls + ".json")
    if not os.path.exists(ip): continue
    rec = load(ip)
    if check(V_INPUT, rec, "input/" + cls) and rec["content_kind"] != kind:
        fail("input/%s: content_kind must be %s" % (cls, kind))
    if rec.get("trust") in ("trusted_runtime", "user_supplied") and cls not in ("runtime-generated", "user-request"):
        fail("input/%s: only runtime-generated and user-request material may be trusted" % cls)
    for k in list(rec):
        bad = {x: v for x, v in rec.items() if x != k}
        if V_INPUT.is_valid(bad): fail("negative control: input/%s without %s validated" % (cls, k))
    if V_INPUT.is_valid({**rec, "trust": "trusted"}): fail("negative control: input/%s with bad trust validated" % cls)
if V_INTENT.is_valid({"kind": "tool_request", "request_id": "r", "tool": {"name": "t"}, "arguments": {},
                      "provenance": {"dialect": "d", "parser_version": "0.1.0", "exposure": {"inputs": [], "floor": "trusted"}}}):
    fail("negative control: exposure with bad floor validated")

# ---- dialect fixtures
n_dial = n_int = 0
for p in sorted(glob.glob(os.path.join(ROOT, "dialects", "fixtures", "*", "*.json"))):
    f = load(p); where = os.path.relpath(p, ROOT); n_dial += 1
    for k in ("input","model","trace_id","turn","expected"):
        if k not in f: fail("%s: missing %s" % (where, k))
    e = f.get("expected", {})
    for k in ("text","intents","rejected","partial","dialect_version"):
        if k not in e: fail("%s: expected missing %s" % (where, k))
    for i, it in enumerate(e.get("intents", [])):
        n_int += 1
        check(V_INTENT, it, "%s intent[%d]" % (where, i))
        if it["provenance"].get("source_digest") is not None: fail("%s: source_digest must be null" % where)
    if V_INTENT.is_valid({**e["intents"][0], "kind": "result"}) if e.get("intents") else False:
        fail("negative control: wrong kind validated")
req = {"openai": {"single","multiple","arguments_not_json","arguments_not_object","missing_name","reasoning_content_present","plain_answer","legacy_function_call"},
       "qwen35": {"valid_single_call","multiple_calls","malformed_unclosed_function","reasoning_plus_call","plain_answer","unknown_tool_name","partial_truncated_call","multiline_parameter","json_typed_parameter","hermes_json"},
       "aien_legacy": {"valid_single_call","multiple_calls","reasoning_plus_call","repaired_bracket","repaired_missing_braces","trailing_comma_rejected","single_quotes_rejected","unterminated_rejected","fenced_json_fallback","placeholder_name_rejected","missing_arguments","arguments_not_object","plain_answer"}}
for dia, names in req.items():
    have = {os.path.basename(x)[:-5] for x in glob.glob(os.path.join(ROOT, "dialects", "fixtures", dia, "*.json"))}
    if have != names: fail("dialect %s fixtures differ: %s" % (dia, sorted(have ^ names)))

print("conformance cases: %d, lifecycle cases: %d, explicit envelopes validated: %d, dialect fixtures: %d, dialect intents validated: %d, digest fixtures: %d, input examples: %d, negative controls: ok" % (n_cases, n_life, n_env, n_dial, n_int, n_dig, len(have_in)))
if errors:
    print("FAIL (%d)" % len(errors)); [print(" -", e) for e in errors]; sys.exit(1)
print("PASS")
