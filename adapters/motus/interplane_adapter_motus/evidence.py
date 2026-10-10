"""Evidence bundle: records, digests, and the bundle digest.

Every digest is sha256 over canonical JSON (RFC 8785, ``interplane.core.digest``), written
``sha256:<hex>``. A record digest covers every field of the record except ``record_digest``. The
bundle digest covers the schema, the trace id, the header and the ordered record digests, so a
change to any field of the bundle changes it.
"""

from __future__ import annotations

from typing import Any

from interplane.core import digest

SCHEMA = "interplane-adapter-motus/evidence/1"
UNAVAILABLE_MOTUS_ID = (
    "not exposed to a Tool by Motus 0.4.3 (Tool.__call__ receives only the argument string); "
    "supply motus_ids from the host"
)


def record_digest(record: dict) -> str:
    return digest({k: v for k, v in record.items() if k != "record_digest"})


def bundle_digest(schema: str, trace_id: str, header: dict, record_digests: list) -> str:
    return digest(
        {
            "schema": schema,
            "trace_id": trace_id,
            "header": header,
            "record_digests": list(record_digests),
        }
    )


def seal_record(record: dict) -> dict:
    record = dict(record)
    record.pop("record_digest", None)
    record["record_digest"] = record_digest(record)
    return record


def build_bundle(trace_id: str, header: dict, records: list) -> dict[str, Any]:
    sealed = [seal_record(r) for r in records]
    return {
        "schema": SCHEMA,
        "trace_id": trace_id,
        "header": header,
        "records": sealed,
        "bundle_digest": bundle_digest(
            SCHEMA, trace_id, header, [r["record_digest"] for r in sealed]
        ),
    }
