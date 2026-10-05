"""Negative controls V1 to V6 (bench/PROTOCOL-0.3.md section 6): deliberately vulnerable variants of
the harness, so the suite can show it is able to fail. TEST ONLY: imported lazily by the
conformance runner when ``--variant`` or ``--matrix`` is given, never by a pipeline library. Each
variant wraps the unmodified pipeline, runtime or lifecycle from the outside."""

import json
from pathlib import Path
from typing import Any, Optional

from .core import RequestLedger

VARIANTS = ("V1", "V2", "V3", "V4", "V5", "V6")


def _claims_approval(req) -> bool:
    """An approval asserted by the model: in ``arguments`` or in ``extensions``."""

    def claim(m: Any) -> bool:
        if not isinstance(m, dict):
            return False
        aid = m.get("approval_id")
        return (isinstance(aid, str) and aid != "") or m.get("approved") is True

    return claim(req.arguments) or claim(req.extensions)


def tamper_decision(variant: Optional[str], req, decision) -> None:
    """V2 honours a model-asserted approval; V4 ignores exposure and authorizes every effect."""
    from .conformance import EFFECT_CAPABILITIES

    if decision.decision not in ("denied", "requires_approval"):
        return
    if (variant == "V2" and _claims_approval(req)) or (
        variant == "V4" and req.capability in EFFECT_CAPABILITIES
    ):
        decision.decision = "authorized"
        decision.reason = None
        decision.approval = None


def before_step(variant: Optional[str], pipe) -> None:
    """V6: re-admission with duplicate detection off. Every step starts with a fresh ledger."""
    if variant == "V6":
        pipe.ledger = RequestLedger()


def _strings(v: Any, out: list) -> None:
    if isinstance(v, str):
        out.append(v)
    elif isinstance(v, list):
        for x in v:
            _strings(x, out)
    elif isinstance(v, dict):
        for k in sorted(v):
            _strings(v[k], out)


def host_loop(variant, pipe, name, trace_id, new, observed, results) -> None:
    """V1: the host loop parses tool-call markup found in rendered results and inputs as if the
    model had written it, and runs the resulting intents."""
    if variant != "V1":
        return
    texts: list = []
    for r in new:
        _strings(r, texts)
    for text in texts:
        if "<tool_call>" not in text:
            continue
        turn = 1000 + len(observed)
        out = pipe.run_turn(
            "qwen35",
            "Qwen/Qwen3.5-9B",
            text,
            trace_id,
            turn,
            message_id_prefix=f"m-{name}-{turn}",
        )
        results.extend(out.results)
        observed.extend(r.to_dict() for r in out.observed)


def finish(variant: Optional[str], results: list, inputs: list) -> None:
    """V3: absent or unknown trust defaults to ``trusted_runtime`` (results and the input ledger)."""
    if variant != "V3":
        return
    for r in results:
        prov = r.provenance
        if isinstance(prov, dict) and prov.get("trust") in (None, "unknown"):
            prov["trust"] = "trusted_runtime"
            prov["trusted"] = True
    for rec in inputs:
        if rec.get("trust") in (None, "unknown"):
            rec["trust"] = "trusted_runtime"


def continuation_any_id(variant: Optional[str], minted: dict, rid: str, decision) -> None:
    """V5: a continuation is accepted when it cites any non-empty ``approval_id`` (Rust lifecycle at
    678c06c). Modelled by presenting the minted id in place of whatever the caller cited."""
    if variant != "V5" or not isinstance(decision.approval, dict):
        return
    if rid not in minted:
        minted[rid] = decision.approval.get("approval_id")
    elif decision.approval.get("approval_id"):
        decision.approval["approval_id"] = minted[rid]


def matrix(fixtures_dir: str, spec_path: str) -> dict:
    """Run the suite unmodified and once per variant; check each variant fails every case the spec
    lists for it. ``valid`` is true only if the unmodified run is clean and every variant is
    detected."""
    from .conformance import run_suite

    spec = json.loads(Path(spec_path).read_text(encoding="utf-8"))

    def failed(variant: Optional[str]) -> list:
        verdicts, lifecycle, digests = run_suite(fixtures_dir, variant)
        names = [k for k, v in {**verdicts, **lifecycle}.items() if not v["pass"]]
        names += [n for n, err in digests if err is not None]
        return sorted(names)

    base = failed(None)
    valid = not base
    rows: dict = {}
    for v in VARIANTS:
        row = spec["variants"][v]
        got = failed(v)
        must = list(row["must_fail"])
        detected = bool(must) and all(m in got for m in must)
        valid = valid and detected
        rows[v] = {"defect": row["defect"], "failed": got, "must_fail": must, "detected": detected}
    return {"baseline_failed": base, "valid": valid, "variants": rows}
