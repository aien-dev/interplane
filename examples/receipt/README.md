# Before/after receipt: CrossAxis Select on a real Qwen3.5 task

This is the 0.1 goal demonstration: a real Qwen3.5 agent task over the real Odysseus tool catalog,
run twice through the same INTERPLANE pipeline, once with every tool exposed and once with the
CrossAxis `domain_match` selection. Every model turn is parsed by Lenshift (`openai` dialect),
admitted by Core, mapped by CrossAxis and decided/executed by Crossveil with `OdysseusAuthority`
(Odysseus's own gates decide, Odysseus's own handlers execute the read-only tools in a throwaway
workspace). The backend was qualified first by Interplane Probe; the probe report is pinned into
the receipt by digest.

## Result (receipts/qwen35-9b-ollama-run1.json and run2.json, 2026-10-04)

| | before (all tools) | after (CrossAxis Select) |
|---|---|---|
| tools in the request | 71 | 9 |
| rendered tool bytes | 59 024 | 5 664 |
| first-turn prompt tokens | 15 846 | 1 674 (89.4% fewer) |
| model rounds to finish | 5 | 4 |
| tool calls decided by Odysseus | 4 | 3 |
| executed for real | 2 (`ls`, `read_file`) | 2 (`ls`, `read_file`) |
| task success | yes | yes |

Backend: `qwen3.5:9b` on Ollama 0.34.0 (GB10, aarch64). Probe profiles: `interplane.core.0.1`,
`lenshift.openai.1`, `relayline.tool_replay.1` compatible; `lenshift.qwen35.1` incompatible on this
endpoint because Ollama's own parser consumes the Qwen3.5 text-form tool call (see
`qualification/reports/qwen35-9b-ollama-0.34.0.probe.json`).

What the receipt shows honestly:
- In the full-surface run the model first called `get_workspace` and then `bash`; Odysseus
  authorized both, but the reference adapter executes only `read_file`, `ls`, `glob`, `grep`, so
  they came back `FAILED / execution_error "not executed by the reference adapter"`. The model
  recovered and finished with `ls` + `read_file`. With the selected surface `bash` was not offered
  (domain `process`), and the run was one round shorter.
- File reads carry `trust = workspace_untrusted`, `content_kind = workspace_content`, because the
  adapter follows Odysseus's treatment of workspace content. Decisions and execution are Odysseus's.
- Tool arguments are never written to the receipt, only their digests and key names.

## Reproducibility

A second independent run (`run2.json`, started with `--verify run1`) reproduced the same tool
selection (71 -> 9), the same first-turn token reduction (15 846 -> 1 674), the same successful
answer, and the same deterministic selection digest. The complete executions were not
byte-identical: totals across all rounds differed slightly (selected path: 7 077 vs 7 083 prompt
tokens, 150 vs 152 completion tokens) and request ids and timings differ by nature. The receipt
separates what must reproduce (`deterministic_digest`) from what happened to.

## Run it

```
ODYSSEUS_SRC=~/workspace/odysseus PYTHONPATH=python:adapters/odysseus \
  <odysseus-venv>/bin/python examples/receipt/run_receipt.py \
  --endpoint http://127.0.0.1:11434/v1 --model qwen3.5:9b \
  --probe qualification/reports/qwen35-9b-ollama-0.34.0.probe.json \
  --out examples/receipt/receipts/my-run.json \
  [--verify examples/receipt/receipts/qwen35-9b-ollama-run1.json]
```

The Odysseus checkout and venv are described in `adapters/odysseus/README.md`. The script needs
the real Odysseus code: without it `decide()` fails closed and nothing executes.
