# Security

INTERPLANE is an interoperability protocol, not a permission system. If you find a way for a valid
INTERPLANE message, signature, retry, replay or dialect quirk to cause execution without an explicit
runtime `authorized` decision, that is a vulnerability in INTERPLANE. Report it privately through
GitHub Security Advisories on this repository. Vulnerabilities in a host runtime's own policy belong
to that project (Odysseus: its SECURITY.md; AIEN: aien-dev).

Rules the reference implementations enforce and the conformance suite tests: fail closed on unknown
decisions, unknown versions, unknown dialects, oversized or malformed arguments, duplicate request
ids and replayed messages; never log secrets; never persist raw model output by default.
