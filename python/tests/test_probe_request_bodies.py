"""Pins probe request bodies so the Python and Rust reference probes cannot drift apart.

The Rust twin is `chat_basic_request_body_is_pinned` in rust/crates/interplane-probe/src/tests.rs.
Both reference probes send identical request bodies per probe id (spec/PROBE.md).
"""
from interplane import probe as p
from interplane.core import digest


class _Recorder(p.Endpoint):
    def __init__(self):
        super().__init__("http://127.0.0.1:1/v1", "m")
        self.sent = []

    def post(self, path, body):
        self.sent.append(body)
        return 500, "{}", "sha256:x", "sha256:y"


def test_chat_basic_request_body_is_pinned():
    ep = _Recorder()
    p.Prober(ep).chat_basic()
    assert ep.sent[0] == {
        "model": "m",
        "messages": [{"role": "user", "content": "Reply with the single word: ready"}],
        "max_tokens": 2048,
        "temperature": 0,
        "seed": 42,
    }
    assert digest(ep.sent[0]) == "sha256:ed2eb0d61ca98c888bf63c7a884f2f568ee4bc54943f98acbe49db86ecdde923"
