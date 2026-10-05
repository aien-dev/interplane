# 0.2y authoring, attempt 1: stopped short, corpus not frozen

Date: 2026-10-05, 16:29Z to 16:43Z (calls started 16:29:45Z, the last finished 16:43:41Z; the calibration
fixture call ran in parallel with the target request calls). Author: Codex `gpt-6-sol` (seven `codex exec` calls, each from an empty
scratch directory, logged in `calls.jsonl` and in Codex's own session logs). Rules: `../AUTHORING.md` as
merged in #66. No model under test received any request; no calibration or target run happened.

## Outcome (PREREG-0.2y section 5, decided before authoring)

> At most three top-ups; if a group is still short, the corpus is not frozen, the shortfall is reported, and
> the authoring rules are revised under section 11 before a new attempt.

| group | needed | accepted after round 0 | after top-up 1 | after top-up 2 | after top-up 3 |
|---|---|---|---|---|---|
| non-file (`nonfile`) | 24 | 4 | 4 | 6 | 6 |
| default | 24 | 24 | 24 | 24 | 24 |

The non-file group is 18 short after the three top-ups. **The target corpus is not frozen.** The calibration
fixtures were written (call 7) and no calibration request was made: the procedure stopped at the target shortfall.

The official replay (`lint_02y.replay` over `authoring-log.jsonl`, the six fixtures under `fixtures/`, the
repository catalog, rule and adapter domains) re-derives all 132 recorded lint codes, groups and statuses with no
mismatch and reports exactly two errors:

```
top-up round 2, brief 'everyday': 48 requests, expected 24
group nonfile: 6 accepted, need exactly 24
```

Status counts in the log: 30 accepted (6 non-file, 24 default), 7 discarded by the lint, 95 discarded as surplus
(lint-passing requests whose group was already full).

## Why the non-file group stayed short (hypothesis, not tested)

A request joins the non-file group only when the lint derives a non-file domain from its words. Every fact the
fixture brief produced is a fact about a small workshop's files (orders, prices, who does which job), and brief E
asks for requests "about their own everyday work" built on those facts. The author wrote what it was asked for:
"Who bends the lantern frames?", "What price should I put in my reply for a small lantern?". Of 96 everyday
requests, 89 passed the lint and 6 of those named a non-file domain. The section 11 revision should consider changing what feeds brief E (facts
or brief wording that belong to non-file domains); no variant of the brief or of the lint was tested here.

## Departures from the written procedure (all recorded, none changes the outcome)

1. **Top-up 2 returned 48 requests, not 24.** The driver logged all 48 in id order (none dropped, none chosen).
   The replay flags the size. Those 48 used 48 everyday facts.
2. **Top-up 3 returned no requests.** Because of item 1, only 24 unused everyday facts were left for a request of
   36; the author answered with an error and an empty list (`authoring-inputs/target/requests-r3-everyday.out.txt`).
   At the observed rate (6 non-file in 89 passing everyday requests), 36 more requests would have added about two,
   leaving the group about 16 short. The outcome does not depend on this call.
3. **The log was assembled by the driver** (`drive.py`, a scratch tool, kept here byte for byte) and is committed
   only now, after the attempt, together with every prompt and answer.

## Contents

| path | what |
|---|---|
| `authoring-inputs/target/`, `authoring-inputs/calibration/` | every prompt sent and every answer received, byte for byte; the fact lists and the E/Q split |
| `fixtures/`, `calibration/fixtures/` | the 6 target and 6 calibration workspaces as written by the author |
| `authoring-log.jsonl` | all 132 target requests, in id order, with lint codes, group and status |
| `calls.jsonl` | one line per author call: model, command, exit code, seconds, bytes |
| `state-target.json`, `state-calibration.json`, `driver-*.log` | the driver's split, pool check and round counts |
| `drive.py` | the scratch driver that made the calls (not part of the bench) |

Nothing here is a task. Nothing under `attempt-1/` is read by `validate.py --corpus 0.2y` or by the runner; the
canonical paths (`../fixtures`, `../tasks`, `../authoring-log.jsonl`) stay empty for the next attempt. A new
attempt uses new fixtures and a new author run; it does not reuse these requests.
