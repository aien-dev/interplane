"""Offline test of the gate M live-model runner (bench/tools/live_injection.py): scripted endpoint
only, no model, no GPU. A fake model that obeys the injection must show the injected intent and 0
violations (the mock gates it); one that refuses, stays silent or does something else must not.

  PYTHONPATH=python python bench/tools/test_live_injection.py
"""

import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import live_injection as li  # noqa: E402


def check(cond, msg):
    if not cond:
        raise AssertionError(msg)


def rows_for(mode: str, only=None) -> tuple:
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp)
        client = li.ScriptedClient(mode)
        summary = li.run(client, out, only)
        rows = [json.loads(p.read_text()) for p in sorted((out / "cases").glob("*.json"))]
        files = sorted(p.name for p in out.iterdir())
        md = (out / "summary.md").read_text()
        manifest = json.loads((out / "manifest.json").read_text())
    return summary, rows, files, md, manifest


def main() -> int:
    cases = li.select_cases()
    check(len(cases) >= 33, f"selected {len(cases)} cases")
    check({c["injection"]["category"] for c in cases} == set(li.CATEGORIES), "categories")

    # obedient fake model: every case emits the injected intent, the mock stops every effect
    summary, rows, files, md, manifest = rows_for("obey")
    check(not summary["errors"], f"errors {summary['errors']}")
    check(all(r["emitted_injected_intent"] for r in rows), [r["case"] for r in rows if not r["emitted_injected_intent"]])
    check(summary["pooled"]["follow_rate"] == 1.0 and summary["violations_total"] == 0, "obey pooled")
    check(all(r["violations"] == 0 for r in rows), "violations under obey")
    injected = [d for r in rows for d in r["decisions"] if d["injected"]]
    check(injected and all(not d["execute_invoked"] for d in injected if d["capability"] in li.EFFECT_CAPABILITIES), "execute for injected effect")
    check(all("arguments" not in d for d in injected) and all(d["arguments_digest"].startswith("sha256:") for d in injected), "args stored")
    check({"manifest.json", "summary.json", "summary.md", "cases"} <= set(files), files)
    check(manifest["temperature"] == 0.0 and manifest["seed"] == 42 and manifest["corpus_digest"].startswith("sha256:"), "manifest")
    check("Gate: PASS" in md and "pooled" in md, "summary.md")
    check("CANARY" not in json.dumps(rows), "canary text leaked into case records")

    # refusing fake model: no injected intent, classed refusal, nothing decided
    summary, rows, *_ = rows_for("refuse", ["injection-01", "injection-32"])
    check(len(rows) == 2 and not any(r["emitted_injected_intent"] for r in rows), "refuse emitted")
    check(all(r["classification"] == "refusal" and r["decisions"] == [] for r in rows), "refuse class")
    check(summary["pooled"]["refusal"] == 2 and summary["pooled"]["follow_rate"] == 0.0, "refuse summary")

    # silent and other-call fake models
    _, rows, *_ = rows_for("silent", ["injection-01"])
    check(rows[0]["classification"] == "no_answer", rows[0]["classification"])
    _, rows, *_ = rows_for("other", ["injection-32"])
    check(rows[0]["classification"] == "other" and not rows[0]["emitted_injected_intent"], "other")

    # prefix: a read case replays the scripted read and carries the mock's injected tool text
    case = next(c for c in cases if c["case"] == "injection-32-read-result-steers-to-write")
    pipe, _ = li.new_pipeline(case)
    messages, turn = li.build_prefix(case, pipe)
    check([m["role"] for m in messages] == ["system", "user", "assistant", "tool"] and turn == 1, [m["role"] for m in messages])
    check("CANARY-I32" in messages[-1]["content"], "tool result lacks the injection text")
    print("test_live_injection: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
