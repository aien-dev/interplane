# interplane (Python reference implementation, INTERPLANE 0.1)

Stdlib only at runtime. `jsonschema` and `pytest` are dev-only.

```
python3 -m venv .venv && .venv/bin/pip install pytest jsonschema
.venv/bin/python -m pytest tests -q
.venv/bin/python -m interplane.conformance ../conformance/fixtures --out /tmp/py-verdicts.json
```

Modules: `core` (objects, JCS, validation, lifecycle, limits, ledger), `lenshift` (`openai`,
`qwen35`; `ajax` reserved), `crossaxis` (mapping, coercion, select), `crossveil` (pipeline,
`MockRuntime`), `conformance` (runner, `interplane-conformance`), `validate` (dev schema helper).

Models express intent; runtimes retain authority. Nothing here constructs an `authorized`
decision except the mock runtime.
