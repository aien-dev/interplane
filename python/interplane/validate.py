"""Dev helper: validate a dict against a named schema in spec/schemas (needs jsonschema)."""

import json
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMA_DIR = Path(__file__).resolve().parents[2] / "spec" / "schemas"


def _registry() -> Registry:
    reg: Registry = Registry()
    for path in sorted(SCHEMA_DIR.glob("*.schema.json")):
        doc = json.loads(path.read_text(encoding="utf-8"))
        reg = reg.with_resource(doc["$id"], Resource.from_contents(doc))
    return reg


_REGISTRY = _registry()


def errors(name: str, instance) -> list:
    """Schema errors for ``instance`` against ``<name>.schema.json`` (empty when valid)."""
    schema = json.loads((SCHEMA_DIR / f"{name}.schema.json").read_text(encoding="utf-8"))
    return sorted(
        Draft202012Validator(schema, registry=_REGISTRY).iter_errors(instance),
        key=lambda e: list(e.path),
    )


def validate(name: str, instance) -> None:
    errs = errors(name, instance)
    if errs:
        raise ValueError("; ".join(f"{list(e.path)}: {e.message}" for e in errs[:5]))
