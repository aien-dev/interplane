#!/usr/bin/env python3
"""Validate INTERPLANE conformance and dialect fixtures against spec/schemas.

Exit status is non-zero on any failure. Checks:
  * every explicit envelope step validates against envelope.schema.json
  * every intent in dialects/fixtures validates against intent.schema.json
  * fixture shape, 18 required cases, filename == case slug
  * expected.runtime counts agree with the observed records
  * the JCS digest fixture recomputes
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
 "14-unknown-fields","15-oversized-arguments","16-duplicate-request-ids","17-replay","18-model-retry-after-denial"}
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
        else: fail("%s: step %s has neither envelope nor dialect+input" % (name, s.get("turn")))
    # size checks for the oversized case
    if name.startswith("15"):
        a = json.loads(f["steps"][0]["input"]["tool_calls"][0]["function"]["arguments"])
        if len(a["content"]) != 70000 or set(a["content"]) != {"a"}: fail("15: content must be 70000 x 'a'")
        if len(jcs(a).encode()) <= 65536: fail("15: arguments not oversized")
if seen != REQUIRED: fail("required cases missing/extra: %s" % sorted(seen ^ REQUIRED))
if not neg_done: fail("negative control never ran")

# ---- digest fixture
dp = os.path.join(FIX, "digest", "jcs-01.json")
d = load(dp)
got = jcs(d["value"])
if got != d["expected_jcs"]: fail("digest/jcs-01: expected_jcs mismatch")
if "sha256:" + hashlib.sha256(got.encode("utf-8")).hexdigest() != d["expected_sha256"]: fail("digest/jcs-01: sha256 mismatch")

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
       "qwen35": {"valid_single_call","multiple_calls","malformed_unclosed_function","reasoning_plus_call","plain_answer","unknown_tool_name","partial_truncated_call","multiline_parameter","json_typed_parameter","hermes_json"}}
for dia, names in req.items():
    have = {os.path.basename(x)[:-5] for x in glob.glob(os.path.join(ROOT, "dialects", "fixtures", dia, "*.json"))}
    if have != names: fail("dialect %s fixtures differ: %s" % (dia, sorted(have ^ names)))

print("conformance cases: %d, explicit envelopes validated: %d, dialect fixtures: %d, dialect intents validated: %d, negative controls: ok" % (n_cases, n_env, n_dial, n_int))
if errors:
    print("FAIL (%d)" % len(errors)); [print(" -", e) for e in errors]; sys.exit(1)
print("PASS")
