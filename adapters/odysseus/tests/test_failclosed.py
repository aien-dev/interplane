"""These run with or without Odysseus: not importable must mean denied, never authorized."""

from interplane_adapter_odysseus import _odysseus
from interplane_adapter_odysseus.authority import OdysseusAuthority
from interplane_adapter_odysseus.demo import make_pipeline, openai_call


def test_unavailable_runtime_denies_everything():
    authority = OdysseusAuthority(odysseus=False)
    assert not authority.available
    pipe = make_pipeline(authority)
    for n, (name, args) in enumerate([("read_file", {"path": "a"}), ("ask_user", {"question": "q", "options": ["a"]}), ("bash", {"command": "ls"})]):
        out = pipe.run_turn("openai", "m", openai_call(name, args, f"c{n}"), "t", n)
        res, rec = out.results[0], out.observed[0]
        assert res.status == "denied" and res.error.message == "odysseus runtime not available"
        assert rec.decision == "denied" and rec.execute_invoked is False


def test_unknown_checkout_is_not_importable():
    assert _odysseus.load("/nonexistent/odysseus") is None
