"""Lenshift: model dialect parsers and result renderers.

Lenshift never executes, never authorizes and never decides whether a capability exists.
The registry maps a dialect name to a module exposing ``parse`` and ``render_result``.
"""

from typing import Any

from ..core import ErrorCode


class UnsupportedDialect(LookupError):
    """Raised by ``get`` for a dialect that is not registered."""

    code = ErrorCode.UNSUPPORTED_DIALECT

    def __init__(self, name: Any):
        super().__init__(f"unsupported_dialect: {name!r}")
        self.dialect = name


class Registry:
    def __init__(self) -> None:
        self._dialects: dict = {}

    def register(self, name: str, module: Any) -> None:
        if not (hasattr(module, "parse") and hasattr(module, "render_result")):
            raise TypeError("a dialect needs parse() and render_result()")
        self._dialects[name] = module

    def get(self, name: Any) -> Any:
        try:
            return self._dialects[name]
        except (KeyError, TypeError):
            raise UnsupportedDialect(name) from None

    def names(self) -> list:
        return sorted(self._dialects)


default_registry = Registry()
register = default_registry.register
get = default_registry.get
names = default_registry.names

from . import aien_legacy, openai, openai_stream, qwen35  # noqa: E402  (registered below; ajax is reserved, not registered)

register("aien_legacy", aien_legacy)
register("openai", openai)
register("openai_stream", openai_stream)
register("qwen35", qwen35)
