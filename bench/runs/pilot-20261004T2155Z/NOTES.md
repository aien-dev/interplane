# Pilot notes (dev tasks only, 7 pairs)

Harness: no defect found in the pilot. Observed backend behaviour, kept as is (protocol section 2/5):
Ollama (qwen3.5:9b, thinking on) sometimes returns the whole final reply in `reasoning` with an empty `content`
(finish_reason stop). The protocol judges the run-ending turn's text (`content`), so such a run has an empty
final answer and fails answer checks. In the pilot this hit condition A twice (ambiguous-001, exec_failure-001;
confirmed by a raw replay: content "" and reasoning "The configured port ... is **8417**"), condition B never.
It is a backend/model behaviour, not a harness defect; the analyzer reports empty final answers per condition.
Dev tasks are excluded from every gate. Concurrent load: llama-server (18081, 43501) and SGLang (18082) were
live on the same GB10 (GPU ~96% before the run), so wall-clock is inflated and noisy.
