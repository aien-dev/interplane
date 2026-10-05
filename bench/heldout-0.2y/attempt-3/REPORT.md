# 0.2y authoring, attempt 3 (last): stopped short, corpus not frozen, 0.2y authoring ends

Date: 2026-10-05, 18:42:52Z to 18:49:54Z. Author: Codex `gpt-6-sol` (six `codex exec` calls, each from an empty
scratch directory, logged in `calls.jsonl` and in Codex's own session logs). Rules: `../AUTHORING.md` as amended in
#71 (PREREG-0.2y section 11, amendment 4: brief E may say which everyday thing a request is about; a top-up states
its count once; attempt 3 is the last). No model under test received any request; no calibration fixtures were
written and no calibration or target run happened.

## Outcome

| group | needed | accepted after round 0 | after top-up 1 | after top-up 2 | after top-up 3 |
|---|---|---|---|---|---|
| non-file (`nonfile`) | 24 | 1 | 2 | 2 | 3 |
| default | 24 | 24 | 24 | 24 | 24 |

The non-file group is 21 short after the three top-ups. **The target corpus is not frozen.** Under amendment 4
item 16 this ends 0.2y authoring: the shortfall is the 0.2y result, the fix under test (section 2) is not evaluated
on these tasks, and any new attempt is a new pre-registration.

The fixtures passed every amendment 3 check (6 workspaces, 120 area-tagged facts on each side, 108 needed per
brief). The official replay (`lint_02y.replay` over `authoring-log.jsonl` and the six fixtures under `fixtures/`)
re-derives all 144 recorded lint codes, groups and statuses with no mismatch and reports exactly one error and no
note:

```
group nonfile: 3 accepted, need exactly 24
```

Status counts: 27 accepted (3 non-file, 24 default), 27 discarded by the lint, 90 discarded as surplus. No
`discarded_over_delivery` and no `empty_call`.

## What amendment 4 changed, and what it did not

- **Top-up count (item 15): fixed.** Each top-up stated its count once and the author returned exactly 12, 24
  and 36 requests (attempt 2: 24 for 12, then a refusal of "72").
- **Brief E wording (item 14): no effect on the shortfall.** The brief sent (`authoring-inputs/target/
  requests-r0-everyday.prompt.txt`) carried the new sentence, which allows a request to say which everyday thing it
  is about ("in my mail", "at the meeting"). The author used it 3 times in 108 everyday requests:
  "Which clapper did Luma choose at the quay meeting?" (accepted, non-file), "In my notes about the bell workshop,
  what is Solen waiting for?" (accepted, non-file), "In my mail with Solen, what color paper did they want for the
  receipt?" (discarded by the lint for `with`). The other 105 were bare questions, as in attempt 2 ("When did Varo
  say he could pick up the bell?"). The third non-file request, "What happened to the old silver clapper plan?",
  derived a non-file domain from an ordinary word. Of 83 lint-passing everyday requests, 3 are non-file.

Observed, not tested: with this author, a brief that *allows* the everyday word does not make the author *use* it.
A brief that requires it would be a different design, and amendment 4 item 16 rules out a further amendment.

Lint discards rose to 27 (attempt 2: 11): 25 everyday and 2 question requests. On the everyday ones the codes
were `LEAK_TOOL_TOKEN` 22, `BANNED_WORD` 4 and `DOMAIN_FILE_KEYWORD` 4 (a request can carry more than one), on
ordinary words such as `ask` 8, `send` 5, `path` 4, `cancel` 3, `with` 2, `tail` 2, `mark` 2.
`AUTHORING.md` 6b already lists this strictness as an open point; the three top-ups were the safeguard.

## Departures from the written procedure

None found. `drive.py` (scratch, kept byte for byte) is the attempt 2 driver plus amendment 4 item 15: in a top-up
it removes the number from the brief's first sentence and asserts the count then appears exactly once.
`authoring-log.jsonl` is `state-target.json`'s `log`, one entry per line with sorted keys.

## Contents

| path | what |
|---|---|
| `authoring-inputs/target/` | every prompt sent and every answer received, byte for byte; the fact list and the E/Q split |
| `fixtures/` | the 6 target workspaces as written by the author |
| `authoring-log.jsonl` | all 144 target entries, in id order, with lint codes, group and status |
| `calls.jsonl` | one line per author call: model, command, exit code, seconds, bytes |
| `state-target.json`, `driver-*.log`, `target.done` | the driver's workspaces, split, pool check, round counts and run times |
| `drive.py` | the scratch driver that made the calls (not part of the bench) |

Nothing here is a task. Nothing under `attempt-3/` is read by `validate.py --corpus 0.2y` or by the runner, which
still reports the canonical corpus as not frozen.
