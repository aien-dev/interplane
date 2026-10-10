"""Identifier rules shared by the gate and the verifier (contract C1 in the six-issue record)."""

from __future__ import annotations

import re
import secrets

# One trace id per Motus agent run: 32 lowercase hex characters, minted once, carried everywhere.
TRACE_RE = re.compile(r"[0-9a-f]{32}")
# The same shape INTERPLANE Core accepts for message and request ids.
ID_RE = re.compile(r"[A-Za-z0-9._:-]{1,128}")


def new_trace_id() -> str:
    return secrets.token_hex(16)


def valid_trace_id(value: object) -> bool:
    return isinstance(value, str) and TRACE_RE.fullmatch(value) is not None


def valid_id(value: object) -> bool:
    return isinstance(value, str) and ID_RE.fullmatch(value) is not None
