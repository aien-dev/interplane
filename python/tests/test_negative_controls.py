"""Negative controls V1 to V8 (bench/PROTOCOL-0.3.md section 6) and the injection judge."""

import subprocess
import sys
from pathlib import Path

from interplane.conformance import load_injection_fixtures, run_case
from conftest import CONF

SPEC = CONF.parent / "negative-controls.json"
PYDIR = Path(__file__).resolve().parents[1] / "interplane"


def test_every_injection_fixture_passes_and_is_judged():
    cases = load_injection_fixtures(str(CONF))
    assert len(cases) >= 3
    for case in cases:
        verdict = run_case(case)
        assert verdict["problems"] == [], case["case"]
        inj = verdict["injection"]
        assert inj["violations"] == 0 and inj["content_derived"] == 0
        assert inj["injected_intents"] >= 1


def test_a_vacuous_injection_case_fails():
    case = load_injection_fixtures(str(CONF))[0]
    case["injection"]["canary"] = "NEVER-SENT"
    case["injection"]["target"]["argument_contains"] = "NEVER-SENT"
    assert any("injection" in p for p in run_case(case)["problems"])


def test_matrix_detects_every_variant():
    from interplane.conformance_negctl import VARIANTS, matrix

    m = matrix(str(CONF), str(SPEC))
    assert m["baseline_failed"] == []
    assert m["valid"] is True, m
    assert sorted(m["variants"]) == sorted(VARIANTS) == ["V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8"]
    for name, row in m["variants"].items():
        assert row["detected"] and row["failed"], name


def test_pipeline_libraries_never_import_the_variants():
    code = (
        "import sys, interplane.core, interplane.crossveil, interplane.crossaxis, "
        "interplane.lenshift, interplane.conformance;"
        "sys.exit(1 if 'interplane.conformance_negctl' in sys.modules else 0)"
    )
    assert subprocess.run([sys.executable, "-c", code]).returncode == 0


def test_pipeline_sources_do_not_mention_the_variants():
    for path in list(PYDIR.glob("*.py")) + list(PYDIR.glob("lenshift/*.py")):
        if path.name in ("conformance.py", "conformance_negctl.py"):
            continue
        text = path.read_text(encoding="utf-8")
        assert "negctl" not in text and "negative control" not in text.lower(), path.name
