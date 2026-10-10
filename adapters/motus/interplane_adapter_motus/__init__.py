"""INTERPLANE adapter for the Motus agent host.

The adapter turns a Motus tool call into an INTERPLANE request, runs it through the real
Pipeline against a RuntimeAuthority, and writes digest-verifiable evidence. It never decides.
"""

__version__ = "0.1.0"
MOTUS_PIN = "lithosai-motus==0.4.3"
