#!/usr/bin/env python3
"""Freeze file for the 0.3 trust corpus (bench/PROTOCOL-0.3.md section 8).

  trust_digest.py --check   recompute and compare with conformance/TRUST-DIGEST.txt (CI)
  trust_digest.py --write   rewrite conformance/TRUST-DIGEST.txt

corpus_digest is the sha256 of the `sha256sum` lines (`<hex>  <path>\\n`, repo-relative paths) of
conformance/fixtures/injection/*.json in LC_ALL=C path order. protocol_sha256 is the sha256 of
bench/PROTOCOL-0.3.md. The T3 and T4 adapter subset lists stay empty until the adapter cuts.
"""
import hashlib, os, sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "conformance", "TRUST-DIGEST.txt")
CORPUS = "conformance/fixtures/injection"
PROTOCOL = "bench/PROTOCOL-0.3.md"

def sha(b): return hashlib.sha256(b).hexdigest()

def compute():
    names = sorted(n for n in os.listdir(os.path.join(ROOT, CORPUS)) if n.endswith(".json"))
    lines = ""
    for n in names:
        with open(os.path.join(ROOT, CORPUS, n), "rb") as f:
            lines += "%s  %s/%s\n" % (sha(f.read()), CORPUS, n)
    with open(os.path.join(ROOT, PROTOCOL), "rb") as f:
        proto = sha(f.read())
    return ("corpus: %s\ncorpus_cases: %d\ncorpus_digest: sha256:%s\nprotocol: %s\nprotocol_sha256: sha256:%s\n"
            "t3_subset: []\nt4_subset: []\n" % (CORPUS, len(names), sha(lines.encode("utf-8")), PROTOCOL, proto))

if __name__ == "__main__":
    want = compute()
    if sys.argv[1:] == ["--write"]:
        open(OUT, "w", encoding="utf-8").write(want); print("wrote", OUT)
    elif sys.argv[1:] == ["--check"]:
        have = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else None
        if have != want:
            print("TRUST-DIGEST.txt is stale or missing; expected:\n" + want); sys.exit(1)
        print("TRUST-DIGEST.txt matches")
    else:
        print(__doc__); sys.exit(2)
