"""Conformance runner: ``interplane-conformance <fixtures_dir> --out <verdicts.json>``."""

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

from .core import Limits
from .crossveil import default_pipeline


def _dig(obj: Any, path: str) -> Any:
    for part in path.split("."):
        if isinstance(obj, list):
            obj = obj[int(part)]
        elif isinstance(obj, dict) and part in obj:
            obj = obj[part]
        else:
            raise KeyError(path)
    return obj


def run_case(case: dict) -> dict:
    """Run one fixture on a fresh pipeline and mock runtime; returns the verdict."""
    pipe = default_pipeline(
        Limits.from_dict(case.get("limits")), case.get("mapping_table", "mock-table")
    )
    name, trace_id = case["case"], case["trace_id"]
    observed: list = []
    results: list = []
    turns: list = []
    for step in case["steps"]:
        if "envelope" in step:
            result, record = pipe.admit_envelope(step["envelope"], turn=step.get("turn"))
            results.append(result)
            observed.append(record.to_dict())
            continue
        out = pipe.run_turn(
            step["dialect"],
            step.get("model", ""),
            step["input"],
            trace_id,
            step["turn"],
            message_id_prefix=f"m-{name}-{step['turn']}",
        )
        results.extend(out.results)
        observed.extend(r.to_dict() for r in out.observed)
        turns.append(out.to_dict())
    runtime = {
        "decide_calls": pipe.runtime.decide_calls,
        "execute_calls": pipe.runtime.execute_calls,
    }

    expected = case["expected"]
    problems: list = []
    exp_obs = expected.get("observed", [])
    if len(exp_obs) != len(observed):
        problems.append(f"observed count {len(observed)} != {len(exp_obs)}")
    for i, (got, want) in enumerate(zip(observed, exp_obs)):
        for key, value in want.items():
            if key == "result_digest" and value is None:
                continue
            if got.get(key) != value:
                problems.append(f"observed[{i}].{key}: {got.get(key)!r} != {value!r}")
    if "turns" in expected and turns != expected["turns"]:
        problems.append(f"turns {turns!r} != {expected['turns']!r}")
    if "runtime" in expected and runtime != expected["runtime"]:
        problems.append(f"runtime {runtime!r} != {expected['runtime']!r}")
    for check in expected.get("result_checks", []):
        match = next((r for r in results if r.request_id == check["request_id"]), None)
        try:
            actual = _dig(match.to_dict(), check["path"])
        except (AttributeError, KeyError, IndexError, ValueError):
            problems.append(f"result_check {check['request_id']} {check['path']}: missing")
            continue
        if actual != check["equals"]:
            problems.append(f"result_check {check['request_id']} {check['path']}: {actual!r}")
    return {
        "pass": not problems,
        "observed": observed,
        "turns": turns,
        "runtime": runtime,
        "problems": problems,
    }


def main(argv: Optional[list] = None) -> int:
    ap = argparse.ArgumentParser(prog="interplane-conformance")
    ap.add_argument("fixtures_dir")
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    verdicts: dict = {}
    for path in sorted(Path(args.fixtures_dir).glob("*.json")):
        case = json.loads(path.read_text(encoding="utf-8"))
        verdict = run_case(case)
        verdicts[case["case"]] = verdict
    failed = 0
    for name, v in sorted(verdicts.items()):
        print(f"{'PASS' if v['pass'] else 'FAIL'}  {name}")
        for p in v["problems"]:
            print(f"      {p}")
        failed += not v["pass"]
    print(f"{len(verdicts) - failed}/{len(verdicts)} passed")
    out = {
        k: {f: v[f] for f in ("pass", "observed", "turns", "runtime")} for k, v in verdicts.items()
    }
    Path(args.out).write_text(
        json.dumps(out, sort_keys=True, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
