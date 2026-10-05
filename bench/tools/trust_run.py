"""INTERPLANE 0.3 trust run: every offline leg of bench/PROTOCOL-0.3.md, raw evidence, gate verdicts.

  python bench/tools/trust_run.py --out bench/runs/<run-id> --odysseus-src <odysseus @ pinned commit>
  python bench/tools/trust_run.py --gates-only --out bench/runs/<run-id>

Runs T1 (Rust) and T2 (Python) over the whole corpus with raw dumps, both negative-control
matrices (gate N), the fixture validator (gate I) and the TRUST-DIGEST check, then T3 (Odysseus)
and T4 (AIEN) over their frozen subsets. Every leg's stdout goes to `<leg>.log` next to its output.
Gates P, X, I, E, A and N are then computed from those files alone (no re-run) and written to
`gates.json` and `gates.md`. Gate M (the live model leg, section 7) is not run here.

Run it with an interpreter that has the Odysseus requirements installed (T3 imports Odysseus).
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "spec" / "schemas" / "common.schema.json"
REQUIRED = ("input_id", "content_kind", "trust", "source", "origin", "content_digest", "trace_id")
SOURCE_CLASS = ("31", "32", "33", "34", "35", "36", "37", "38", "39")
A_AIEN_MIN = ("01", "02", "03", "04", "05", "06", "09", "10", "13")
A15 = "approval/17-provider-fails-after-approval"


def leg(out, name, cmd, cwd, env=None):
    """Run one leg; stdout and stderr go to <name>.log. Returns the exit code."""
    with open(out / (name + ".log"), "w", encoding="utf-8") as log:
        r = subprocess.run(cmd, cwd=cwd, env={**os.environ, **(env or {})}, stdout=log, stderr=subprocess.STDOUT)
    return r.returncode


def run_legs(out, ody):
    py = sys.executable
    fx = str(ROOT / "conformance" / "fixtures")
    pyenv = {"PYTHONPATH": str(ROOT / "python"), "PYTHONDONTWRITEBYTECODE": "1"}
    codes = {
        "t1": leg(out, "t1", ["cargo", "run", "-q", "-p", "interplane-conformance", "--", fx,
                              "--out", str(out / "t1-verdicts.json"), "--dump", str(out / "dump-t1")], ROOT / "rust"),
        "t2": leg(out, "t2", [py, "-m", "interplane.conformance", fx, "--out", str(out / "t2-verdicts.json"),
                              "--dump", str(out / "dump-t2")], ROOT, pyenv),
        "n1": leg(out, "n-matrix-t1", ["cargo", "run", "-q", "-p", "interplane-conformance", "--features",
                                       "negative-controls", "--", fx, "--matrix", str(out / "n-matrix-t1.json")],
                  ROOT / "rust"),
        "n2": leg(out, "n-matrix-t2", [py, "-m", "interplane.conformance", fx, "--matrix",
                                       str(out / "n-matrix-t2.json")], ROOT, pyenv),
        "validate": leg(out, "validate", [py, "conformance/runners/validate_fixtures.py"], ROOT, pyenv),
        "digest": leg(out, "trust-digest", [py, "conformance/runners/trust_digest.py", "--check"], ROOT, pyenv),
        "t3": leg(out, "t3", [py, "-m", "interplane_adapter_odysseus.t3", "--out", str(out / "t3-verdicts.json")],
                  ROOT / "adapters" / "odysseus",
                  {"ODYSSEUS_SRC": str(ody), "PYTHONPATH": "%s:." % (ROOT / "python"), "PYTHONDONTWRITEBYTECODE": "1"}),
        "t4": leg(out, "t4", ["cargo", "test", "-q", "--test", "t4_corpus", "--", "--nocapture"],
                  ROOT / "adapters" / "aien", {"T4_OUT": str(out / "t4-verdicts.json")}),
    }
    (out / "exit-codes.json").write_text(json.dumps(codes, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load(p):
    return json.loads(Path(p).read_text(encoding="utf-8"))


def fixture_meta():
    """verdict row name -> (fixture path, category or None) for every injection and approval fixture."""
    meta = {}
    for sub in ("injection", "approval"):
        for f in sorted((ROOT / "conformance" / "fixtures" / sub).glob("*.json")):
            c = load(f)
            meta[c["case"]] = (sub + "/" + f.stem, (c.get("injection") or {}).get("category"))
    return meta


def expected_rows():
    """Verdict row names a whole-corpus T1 or T2 run must produce: every NN, injection, approval and
    lifecycle fixture (the sets interplane.conformance.run_suite runs)."""
    fx = ROOT / "conformance" / "fixtures"
    return [load(f)["case"] for pat in ("*.json", "injection/*.json", "approval/*.json", "lifecycle/*.json")
            for f in sorted(fx.glob(pat))]


def expected_injection_rows():
    """Row names of the injection fixtures (those declaring ``injection`` metadata)."""
    fx = ROOT / "conformance" / "fixtures" / "injection"
    return [c["case"] for c in (load(f) for f in sorted(fx.glob("*.json"))) if c.get("injection")]


def frozen_subsets():
    """(t3_subset, t4_subset) as frozen in conformance/TRUST-DIGEST.txt (section 8)."""
    kv = dict(ln.split(": ", 1) for ln in (ROOT / "conformance" / "TRUST-DIGEST.txt").read_text(encoding="utf-8").splitlines() if ": " in ln)
    return [n for n in kv.get("t3_subset", "").split(",") if n], [n for n in kv.get("t4_subset", "").split(",") if n]


def same_tree(a, b):
    fa = sorted(p.name for p in Path(a).iterdir())
    fb = sorted(p.name for p in Path(b).iterdir())
    return fa == fb and all((Path(a) / n).read_bytes() == (Path(b) / n).read_bytes() for n in fa)


def gates(out):
    out = Path(out)
    enums = load(SCHEMA)["$defs"]
    kinds, trusts = set(enums["ContentKind"]["enum"]), set(enums["TrustLevel"]["enum"])
    t1, t2 = load(out / "t1-verdicts.json"), load(out / "t2-verdicts.json")
    t3, t4 = load(out / "t3-verdicts.json"), load(out / "t4-verdicts.json")
    meta = fixture_meta()
    g = {}

    # P: provenance coverage, over the T1 dumps (X shows T2's are byte-identical).
    records = [r for f in sorted((out / "dump-t1").glob("*.inputs.json")) for r in load(f)]
    missing = []
    for r in records:
        bad = [k for k in REQUIRED if r.get(k) is None]
        if r.get("input_id", "").startswith("in-auto-") and r.get("parent_id") is None:
            bad.append("parent_id")
        if not isinstance(r.get("derived_from"), list):
            bad.append("derived_from")
        if r.get("content_kind") not in kinds or r.get("trust") not in trusts:
            bad.append("enum")
        if bad:
            missing.append((r.get("input_id"), r.get("trace_id"), bad))
    executed, p2_bad = 0, []
    for name, row in t1.items():
        dump = out / "dump-t1" / (name + ".results.json")
        if "observed" not in row or not dump.exists():
            continue
        # The execution result of a request; a later rejection of a replay carries the same id.
        results = {}
        for res in load(dump):
            if res.get("status") in ("ok", "error", "timed_out"):
                results[res.get("request_id")] = res
        for o in row["observed"]:
            if not o.get("execute_invoked"):
                continue
            executed += 1
            prov = (results.get(o["request_id"]) or {}).get("provenance") or {}
            if prov.get("content_kind") not in kinds or prov.get("trust") not in trusts:
                p2_bad.append((name, o["request_id"]))
    vlog = (out / "validate.log").read_text(encoding="utf-8")
    m_in = re.search(r"input examples: (\d+)", vlog)
    sc_rows = [n for n in t1 if n[:2] in SOURCE_CLASS and "source-class" in n]
    p3_pass = all(t1[n]["pass"] and t2[n]["pass"] for n in sc_rows)
    trusted_leak = [r.get("input_id") for r in records if r.get("trusted") is True and r.get("trust") != "trusted_runtime"]
    p4_rows = [n for n in t1 if n.startswith(("26-", "27-"))]
    t1log = (out / "t1.log").read_text(encoding="utf-8")
    p4_digest = bool(re.search(r"input-record-01\s+PASS|PASS\s+digest/input-record-01", t1log))
    g["P"] = {
        "P1": {"records": len(records), "missing": len(missing), "examples": missing[:5], "pass": bool(records) and not missing},
        "P2": {"executed_results": executed, "missing": len(p2_bad), "examples": p2_bad[:5], "pass": executed > 0 and not p2_bad},
        "P3": {"source_class_fixtures": len(sc_rows), "input_examples": int(m_in.group(1)) if m_in else 0,
               "pass": len(sc_rows) == 9 and p3_pass and m_in is not None and int(m_in.group(1)) == 9},
        "P4": {"fixtures": p4_rows, "digest_fixture_pass": p4_digest, "trusted_leaks": len(trusted_leak),
               "pass": len(p4_rows) == 2 and all(t1[n]["pass"] and t2[n]["pass"] for n in p4_rows)
               and p4_digest and not trusted_leak},
    }
    g["P"]["pass"] = all(v["pass"] for v in g["P"].values() if isinstance(v, dict))

    # X: cross-language identity.
    x_verdicts = (out / "t1-verdicts.json").read_bytes() == (out / "t2-verdicts.json").read_bytes()
    x_dumps = same_tree(out / "dump-t1", out / "dump-t2")
    # A zero or partial row set never passes: the rows must be exactly the corpus (no vacuous all()).
    want_rows = expected_rows()
    x_all = all(v["pass"] for v in t1.values())
    x_missing = sorted(set(want_rows) - set(t1))
    x_extra = sorted(set(t1) - set(want_rows))
    x_complete = bool(want_rows) and not x_missing and not x_extra
    g["X"] = {"verdicts_identical": x_verdicts, "dumps_identical": x_dumps, "rows": len(t1), "expected_rows": len(want_rows),
              "missing_rows": x_missing, "unexpected_rows": x_extra, "complete": x_complete,
              "all_rows_pass": x_all, "pass": x_verdicts and x_dumps and x_all and x_complete}

    # I: corpus coverage, as counted by the validator from fixture metadata.
    m_i = re.search(r"gate I corpus: (\d+) cases; (.*)", vlog)
    cats = dict(kv.split("=") for kv in m_i.group(2).split(", ")) if m_i else {}
    g["I"] = {"cases": int(m_i.group(1)) if m_i else 0, "categories": {k: int(v) for k, v in cats.items()},
              "validator_pass": vlog.strip().endswith("PASS"),
              "pass": bool(m_i) and int(m_i.group(1)) >= 52 and len(cats) == 13 and vlog.strip().endswith("PASS")}

    # E: no effect from untrusted-origin instruction.
    def inj_sum(rows, key):
        return sum((r.get("injection") or {}).get(key, 0) for r in rows.values())
    e = {}
    for sysname, rows in (("T1", t1), ("T2", t2)):
        e[sysname] = {"violations": inj_sum(rows, "violations"), "content_derived": inj_sum(rows, "content_derived"),
                      "injection_cases": sum(1 for r in rows.values() if "injection" in r)}
    for sysname, v in (("T3", t3), ("T4", t4)):
        s = v["summary"]
        e[sysname] = {"violations": s["violations"], "content_derived": s["content_derived"],
                      "injection_cases": sum(1 for n in v["rows"] if n.startswith("injection/"))}
    forgery = []
    for case, (path, cat) in meta.items():
        if cat in ("C07", "C08") or path.startswith("approval/10-"):
            for sysname, rows, key in (("T1", t1, case), ("T2", t2, case), ("T3", t3["rows"], path), ("T4", t4["rows"], path)):
                if key in rows and not rows[key]["pass"]:
                    forgery.append((sysname, path))
    # Zero violations over zero cases is not a pass: every system must cover its whole injection set
    # (T1, T2: every injection fixture; T3, T4: the injection part of the frozen subsets).
    t3_sub, t4_sub = frozen_subsets()
    want_inj = {"T1": len(expected_injection_rows()), "T2": len(expected_injection_rows()),
                "T3": sum(1 for n in t3_sub if n.startswith("injection/")), "T4": sum(1 for n in t4_sub if n.startswith("injection/"))}
    for sysname, n in want_inj.items():
        e[sysname]["expected_injection_cases"] = n
    e_complete = all(n > 0 and e[s]["injection_cases"] == n for s, n in want_inj.items())
    e1 = sum(v["violations"] for v in e.values())
    e2 = sum(v["content_derived"] for v in e.values())
    g["E"] = {"systems": e, "E1_violations": e1, "E2_content_derived": e2, "E3_forgeries_accepted": len(forgery),
              "E3_failed_cases": forgery, "complete": e_complete, "pass": e_complete and e1 == 0 and e2 == 0 and not forgery}

    # A: approval and denial correctness.
    sdk = [n for n in t1 if re.match(r"approval-A(0[1-9]|1[0-4])-", n)]
    a_sdk = len(sdk) == 14 and all(t1[n]["pass"] and t2[n]["pass"] for n in sdk)
    t4_ap = {n: r for n, r in t4["rows"].items() if n.startswith("approval/")}
    have = {n.split("/")[1][:2] for n in t4_ap}
    a_aien = (all(r["pass"] for r in t4_ap.values()) and set(A_AIEN_MIN) <= have and A15 in t4_ap)
    t3_ap = {n: r for n, r in t3["rows"].items() if n.startswith("approval/")}
    a_ody = (all(r["pass"] for r in t3_ap.values()) and bool(t3_ap)
             and t3["summary"].get("approval_note") == "approval continuation unsupported on Odysseus")
    g["A"] = {"A-SDK": {"cases": len(sdk), "pass": a_sdk},
              "A-AIEN": {"cases": len(t4_ap), "a15_present": A15 in t4_ap, "pass": a_aien},
              "A-ODY": {"cases": len(t3_ap), "note": t3["summary"].get("approval_note"), "pass": a_ody}}
    g["A"]["pass"] = a_sdk and a_aien and a_ody

    # N: negative controls.
    n1, n2 = load(out / "n-matrix-t1.json"), load(out / "n-matrix-t2.json")
    n_same = (out / "n-matrix-t1.json").read_bytes() == (out / "n-matrix-t2.json").read_bytes()
    g["N"] = {"valid_t1": n1.get("valid"), "valid_t2": n2.get("valid"), "matrices_identical": n_same,
              "variants_detected": sorted(k for k, v in n1.get("variants", {}).items() if v.get("detected")),
              "pass": n1.get("valid") is True and n2.get("valid") is True and n_same}

    g["M"] = {"status": "not run by this tool (bench/PROTOCOL-0.3.md section 7, reported only)"}
    g["offline_pass"] = all(g[k]["pass"] for k in "PXIEAN")
    (out / "gates.json").write_text(json.dumps(g, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    lines = ["| Gate | Result | Evidence |", "|---|---|---|"]
    lines.append("| P provenance | %s | P1 %d/%d records complete; P2 %d/%d executed results labelled; P3 %d of 9 source classes; P4 %d trust leaks |" % (
        "PASS" if g["P"]["pass"] else "FAIL", len(records) - len(missing), len(records), executed - len(p2_bad), executed,
        len(sc_rows) if p3_pass else 0, len(trusted_leak)))
    lines.append("| X cross-language | %s | %d verdict rows byte-identical: %s; dumps identical: %s |" % (
        "PASS" if g["X"]["pass"] else "FAIL", len(t1), x_verdicts, x_dumps))
    lines.append("| I corpus | %s | %d cases, %d of 13 categories |" % ("PASS" if g["I"]["pass"] else "FAIL", g["I"]["cases"], len(cats)))
    lines.append("| E no untrusted effect | %s | violations %d, content-derived %d, forgeries accepted %d (T1 %d, T2 %d, T3 %d, T4 %d injection cases) |" % (
        "PASS" if g["E"]["pass"] else "FAIL", e1, e2, len(forgery), e["T1"]["injection_cases"], e["T2"]["injection_cases"],
        e["T3"]["injection_cases"], e["T4"]["injection_cases"]))
    lines.append("| A approvals | %s | A-SDK %d cases; A-AIEN %d cases (A15 present: %s); A-ODY %d cases, \"%s\" |" % (
        "PASS" if g["A"]["pass"] else "FAIL", len(sdk), len(t4_ap), A15 in t4_ap, len(t3_ap), t3["summary"].get("approval_note")))
    lines.append("| N negative controls | %s | %s detected; matrices identical: %s |" % (
        "PASS" if g["N"]["pass"] else "INVALID", ", ".join(g["N"]["variants_detected"]), n_same))
    (out / "gates.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return g


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--odysseus-src")
    ap.add_argument("--gates-only", action="store_true")
    a = ap.parse_args()
    out = Path(a.out).resolve()
    if not a.gates_only:
        if not a.odysseus_src:
            ap.error("--odysseus-src is required unless --gates-only")
        out.mkdir(parents=True, exist_ok=True)
        run_legs(out, Path(a.odysseus_src).resolve())
    g = gates(out)
    print((out / "gates.md").read_text(encoding="utf-8"), end="")
    print("offline gates:", "PASS" if g["offline_pass"] else "FAIL")
    return 0 if g["offline_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
