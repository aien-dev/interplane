#!/usr/bin/env python3
"""The T3 and T4 subset rule and fixture translation (bench/PROTOCOL-0.3.md section 2; the normative
text is spec/CORE.md, "Adapter subsets"). Pure standard library, no adapter imports.

  adapter_subset.py --subset odysseus|aien   print the subset (one fixture per line)
  adapter_subset.py --plans odysseus|aien    print the translated plans of the subset as JSON

The rule reads only fixture metadata and conformance/adapter-translation.json, never a result.
Both runners (adapters/odysseus T3, adapters/aien T4) execute exactly these plans.
"""
import copy, hashlib, json, os, re, sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
FIXTURES = os.path.join(ROOT, "conformance", "fixtures")
TABLE = os.path.join(ROOT, "conformance", "adapter-translation.json")
QWEN = re.compile(r"<tool_call>\s*<function=(\w+)>(.*?)</function>\s*</tool_call>", re.S)
PARAM = re.compile(r"<parameter=(\w+)>\n?(.*?)\n?</parameter>", re.S)


def load_table():
    with open(TABLE, encoding="utf-8") as f:
        return json.load(f)


def fixtures():
    """(name, case) for the injection and approval fixtures, `injection/NN-slug` order then approval."""
    out = []
    for sub in ("injection", "approval"):
        d = os.path.join(FIXTURES, sub)
        for n in sorted(os.listdir(d)):
            if n.endswith(".json"):
                with open(os.path.join(d, n), encoding="utf-8") as f:
                    out.append(("%s/%s" % (sub, n[:-5]), json.load(f)))
    return out


def _calls(step):
    """[(capability, shape)] a step calls, or None when the step is not an openai, qwen35 or envelope call."""
    if "envelope" in step:
        return [(step["envelope"].get("payload", {}).get("tool", {}).get("name"), "envelope")]
    if "dialect" in step:
        if step["dialect"] == "openai":
            tc = step["input"].get("tool_calls") or []
            return [(c.get("function", {}).get("name"), "openai") for c in tc]
        if step["dialect"] == "qwen35":
            return [(m.group(1), "qwen35") for m in QWEN.finditer(step["input"])]
        return None
    return []


def decide_subset(name, case, adapter, table):
    """(in_subset, reason). The rule, in order; the first failing clause names the reason."""
    ad = table["adapters"][adapter]
    is_approval = name.startswith("approval/")
    if any(k in case for k in table["harness_keys"]):
        return False, "carries a harness-only mock key"
    for s in case["steps"]:
        if any(k in s for k in table["harness_steps"]):
            return False, "has a harness-only mock step"
    if is_approval:
        num = int(name.split("/")[1][:2])
        if not table["approval_cases"]["first"] <= num <= table["approval_cases"]["last"]:
            return False, "not a protocol approval case (A01 to A14)"
        classes = ad["approval_effects"]
    else:
        target = case["injection"]["target"]["capability"]
        if target not in ad["effects"]:
            return False, "target capability class %s not in the adapter catalog" % target
        classes = ad["effects"]
    for s in case["steps"]:
        calls = _calls(s)
        if calls is None:
            return False, "dialect step the rule cannot translate"
        for cap, shape in calls:
            if cap in table["mock_only_capabilities"]:
                return False, "uses mock-only capability %s" % cap
            if cap in classes or cap in ad["native_reads"]:
                continue
            if cap in table["delivered_reads"] and shape != "envelope":
                continue
            return False, "capability %s has no counterpart" % cap
    return True, None


def subset(adapter, table=None):
    table = table or load_table()
    return [n for n, c in fixtures() if decide_subset(n, c, adapter, table)[0]]


def _map_args(spec, args):
    """Translate an argument object by one adapter mapping; keys the mapping does not consume stay."""
    out, used = {}, set()
    for key, m in spec["args"].items():
        keys = m.get("from", [key])
        used.update(keys)
        val = next((args[k] for k in keys if k in args), None)
        if val is None:
            val = m.get("const", m.get("default"))
        if val is None:
            continue
        if m.get("as") == "basename" and isinstance(val, str):
            val = val.rsplit("/", 1)[-1]
        out[key] = val
    out.update({k: v for k, v in args.items() if k not in used})
    return out


def _target(cap, ad, classes):
    if cap in classes:
        return classes[cap]
    return ad["native_reads"].get(cap)


def _digest(obj):
    return "sha256:" + hashlib.sha256(json.dumps(obj, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _delivery(cap, rid, case, table):
    lab = table["delivered_reads"][cap]
    content = (case.get("mock_data", {}).get(cap) or {}).get("data", "mock-delivered:" + cap)
    return {"host_input": {"input": {
        "input_id": "in-delivered-" + rid, "content_kind": lab["content_kind"], "trust": lab["trust"],
        "source": {"kind": "runtime", "id": "host-delivery"}, "origin": "runtime:delivery:" + cap,
        "content_digest": _digest(content), "trace_id": case["trace_id"], "parent_id": None, "derived_from": []}}}


def _xlate_step(step, case, ad, classes, table, plan):
    """Translate one call step. Calls the adapter lacks as read tools are dropped and queued as
    host-registered deliveries, appended to `plan` after the step itself."""
    deliveries = []
    s = copy.deepcopy(step)
    if "envelope" in s:
        p = s["envelope"]["payload"]
        spec = _target(p["tool"]["name"], ad, classes)
        p["tool"]["name"] = spec["name"]
        if isinstance(p.get("provenance"), dict):
            p["provenance"]["raw_name"] = spec["name"]
        p["arguments"] = _map_args(spec, p.get("arguments") or {})
        s["envelope"]["destination"] = {"kind": "runtime", "id": ad["runtime_id"]}
        plan.append(s)
        return
    if s["dialect"] == "openai":
        kept = []
        for i, c in enumerate(s["input"].get("tool_calls") or []):
            cap = c["function"]["name"]
            if cap in table["delivered_reads"] and cap not in classes and cap not in ad["native_reads"]:
                deliveries.append(_delivery(cap, c.get("id") or "t%dc%d" % (s["turn"], i), case, table))
                continue
            spec = _target(cap, ad, classes)
            c["function"]["name"] = spec["name"]
            try:
                args = json.loads(c["function"]["arguments"])
            except ValueError:
                args = None
            if isinstance(args, dict):
                c["function"]["arguments"] = json.dumps(_map_args(spec, args))
            kept.append(c)
        s["input"]["tool_calls"] = kept or None
    else:
        idx = [0]

        def sub(m):
            cap, i = m.group(1), idx[0]
            idx[0] += 1
            if cap in table["delivered_reads"] and cap not in classes and cap not in ad["native_reads"]:
                deliveries.append(_delivery(cap, "%s:t%d:c%d" % (case["trace_id"], s["turn"], i), case, table))
                return ""
            spec = _target(cap, ad, classes)
            args = _map_args(spec, {k: v for k, v in PARAM.findall(m.group(2))})
            body = "".join("<parameter=%s>\n%s\n</parameter>\n" % (k, v) for k, v in args.items())
            return "<tool_call>\n<function=%s>\n%s</function>\n</tool_call>" % (spec["name"], body)

        s["input"] = QWEN.sub(sub, s["input"]).strip()
    plan.append(s)
    plan.extend(deliveries)


def _approval_id(a):
    m = re.fullmatch(r"mock-approval-(.+)", a.get("approval_id") or "")
    return {"minted_for": m.group(1)} if m else {"literal": a.get("approval_id") or ""}


def translate(name, case, adapter, table=None):
    table = table or load_table()
    ad = table["adapters"][adapter]
    is_approval = name.startswith("approval/")
    classes = ad["approval_effects"] if is_approval else ad["effects"]
    plan = []
    for step in case["steps"]:
        if "approve" in step or "cancel" in step:
            kind = "approve" if "approve" in step else "cancel"
            a = dict(step[kind])
            if kind == "approve":
                a["approval_id"] = _approval_id(a)
                if "arguments" in a:
                    spec = classes["delete_file"]
                    a["arguments"] = _map_args(spec, a["arguments"])
            plan.append({kind: a})
        elif "restart" in step:
            plan.append({"restart": True})
        elif "envelope" in step or "dialect" in step:
            _xlate_step(step, case, ad, classes, table, plan)
        else:
            plan.append({"host_input": copy.deepcopy(step)})
    out = {"name": name, "case": case["case"], "kind": "approval" if is_approval else "injection",
           "trace_id": case["trace_id"], "steps": plan}
    if is_approval:
        out["expires_at"] = case.get("mock_approval", {}).get("expires_at")
        out["expected"] = {"observed": case["expected"]["observed"],
                           "continuations": case["expected"].get("continuations", [])}
    else:
        inj = case["injection"]
        out["target"] = {"capability": classes[inj["target"]["capability"]]["name"],
                         "argument_contains": inj["target"]["argument_contains"]}
    return out


def plans(adapter):
    table = load_table()
    byname = dict(fixtures())
    return [translate(n, byname[n], adapter, table) for n in subset(adapter, table)]


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] in ("--subset", "--plans") and sys.argv[2] in ("odysseus", "aien"):
        if sys.argv[1] == "--subset":
            print("\n".join(subset(sys.argv[2])))
        else:
            json.dump(plans(sys.argv[2]), sys.stdout, sort_keys=True)
    else:
        print(__doc__)
        sys.exit(2)
