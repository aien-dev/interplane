#!/usr/bin/env python3
"""Freeze file for the 0.3 trust corpus (bench/PROTOCOL-0.3.md section 8).

  trust_digest.py --check   recompute and compare with conformance/TRUST-DIGEST.txt (CI)
  trust_digest.py --write   rewrite conformance/TRUST-DIGEST.txt

corpus_digest is the sha256 of the `sha256sum` lines (`<hex>  <path>\\n`, repo-relative paths) of
every conformance/fixtures/**/*.json in LC_ALL=C path order (the whole corpus the negative-control
matrix depends on, not only the injection cases). negative_controls_sha256 freezes
conformance/negative-controls.json. protocol_sha256 is the sha256 of
bench/PROTOCOL-0.3.md. adapter_translation_sha256 freezes conformance/adapter-translation.json (the
data of the subset rule), and t3_subset and t4_subset list the fixtures each adapter must run,
computed from fixture metadata by adapter_subset.py (spec/CORE.md, Adapter subsets).
"""
import hashlib, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import adapter_subset

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "conformance", "TRUST-DIGEST.txt")
CORPUS = "conformance/fixtures"
NEGCTL = "conformance/negative-controls.json"
PROTOCOL = "bench/PROTOCOL-0.3.md"
TRANSLATION = "conformance/adapter-translation.json"

def sha(b): return hashlib.sha256(b).hexdigest()

def compute():
    names = sorted(os.path.relpath(os.path.join(d, n), os.path.join(ROOT, CORPUS)).replace(os.sep, "/")
                   for d, _, fs in os.walk(os.path.join(ROOT, CORPUS)) for n in fs if n.endswith(".json"))
    names = sorted(names, key=lambda n: n.encode("utf-8"))
    lines = ""
    for n in names:
        with open(os.path.join(ROOT, CORPUS, n), "rb") as f:
            lines += "%s  %s/%s\n" % (sha(f.read()), CORPUS, n)
    with open(os.path.join(ROOT, NEGCTL), "rb") as f:
        negctl = sha(f.read())
    with open(os.path.join(ROOT, PROTOCOL), "rb") as f:
        proto = sha(f.read())
    with open(os.path.join(ROOT, TRANSLATION), "rb") as f:
        trans = sha(f.read())
    t3, t4 = adapter_subset.subset("odysseus"), adapter_subset.subset("aien")
    return ("corpus: %s\ncorpus_files: %d\ncorpus_digest: sha256:%s\nnegative_controls: %s\nnegative_controls_sha256: sha256:%s\n"
            "protocol: %s\nprotocol_sha256: sha256:%s\nadapter_translation: %s\nadapter_translation_sha256: sha256:%s\n"
            "t3_subset_count: %d\nt3_subset: %s\nt4_subset_count: %d\nt4_subset: %s\n"
            % (CORPUS, len(names), sha(lines.encode("utf-8")), NEGCTL, negctl, PROTOCOL, proto,
               TRANSLATION, trans, len(t3), ",".join(t3), len(t4), ",".join(t4)))

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
