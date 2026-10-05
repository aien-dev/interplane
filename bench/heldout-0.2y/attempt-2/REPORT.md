# 0.2y authoring, attempt 2: stopped short, corpus not frozen

Date: 2026-10-05, 18:23:50Z to 18:30:50Z (fixture call first, then the request calls one after another). Author:
Codex `gpt-6-sol` (six `codex exec` calls, each from an empty scratch directory, logged in `calls.jsonl` and in
Codex's own session logs). Rules: `../AUTHORING.md` as amended in #69 (PREREG-0.2y section 11, amendment 3: the
E/Q split follows the fixture author's area tags; over-delivery is `discarded_over_delivery`; an empty answer is
one `empty_call` line). No model under test received any request; no calibration fixtures were written and no
calibration or target run happened.

## Outcome (PREREG-0.2y section 5, decided before authoring)

> At most three top-ups; if a group is still short, the corpus is not frozen, the shortfall is reported, and
> the authoring rules are revised under section 11 before a new attempt.

| group | needed | accepted after round 0 | after top-up 1 | after top-up 2 | after top-up 3 |
|---|---|---|---|---|---|
| non-file (`nonfile`) | 24 | 2 | 3 | 3 | 3 |
| default | 24 | 24 | 24 | 24 | 24 |

The non-file group is 21 short after the three top-ups. **The target corpus is not frozen.**

The fixtures passed every amendment 3 check: 6 workspaces, 120 area-tagged facts on each side of the split
(108 needed per brief), every workspace with at least 20 facts per side and at least 3 areas.

The official replay (`lint_02y.replay` over `authoring-log.jsonl`, the six fixtures under `fixtures/`, the
repository catalog, rule and adapter domains) re-derives all 121 recorded lint codes, groups and statuses with no
mismatch and reports exactly one error and one note:

```
error: group nonfile: 3 accepted, need exactly 24
note:  top-up round 3, brief 'everyday': 0 requests returned, 36 asked
```

Status counts in the log: 27 accepted (3 non-file, 24 default), 11 discarded by the lint (all
`LEAK_TOOL_TOKEN`, on ordinary words such as "with", "ask", "patch"), 70 discarded as surplus (lint-passing
requests whose group was already full), 12 `discarded_over_delivery`, 1 `empty_call`.

## Why the non-file group stayed short (observed, not yet tested as a fix)

Amendment 3 changed what feeds brief E, and that part worked: brief E received only facts the fixture author
tagged mail, meetings, contacts, to-dos, notes or chats. The requests built on them stayed in the default group.
Lint-passing everyday and question requests by the area of the fact they used:

| fact area | default | non-file |
|---|---|---|
| meetings | 18 | 2 |
| mail | 13 | 0 |
| contacts | 12 | 0 |
| to-dos | 6 | 0 |
| chats | 6 | 0 |
| notes | 5 | 0 |
| other (work files) | 34 | 1 |

A request joins the non-file group only when the lint derives exactly one non-file domain **from the request's
own words** (`lint_02y.py`: `derive_match(request, rule)`; no keyword gives the default list). Brief E ends with
"Do not tell the assistant where to look and do not describe how to find the answer." The author obeyed: "Who
approved the blue enamel sample?" (a mail fact), "Who is our quay courier?" (a contacts fact). The three that did
land ("Where is Monday's color session happening now?", "What event is the cloud sail reserved for?", "Where did
the Tuesday layout session move?") carry a meeting or booking word by chance. The brief forbids the words the
group rule needs. The section 11 revision has to resolve this conflict (for example: a request may name the kind
of thing it is about, "my mail", "the meeting", without naming a file, path or tool). No variant was tested here.

## Second wording defect: the top-up asks for twice the number

`AUTHORING.md` section 4.3 puts `{K}` in the brief ("Write {K} requests ...") and again in the inserted
paragraph ("Write {K} more requests ..."). The author reads the two as a total of 2K:

1. **Top-up 1 asked 12 and got 24.** Per amendment 3 the first 12 were counted (1 accepted, 4 lint, 7 surplus)
   and the other 12 logged as `discarded_over_delivery`, their facts unused.
2. **Top-up 3 asked 36 and got none.** The answer (`authoring-inputs/target/requests-r3-everyday.out.txt`):
   "There are 48 listed facts, so I cannot write 72 requests while using each fact at most once." The driver
   logged one `empty_call`. At the observed rate (2 non-file among 62 lint-passing everyday requests), 36 more
   would have added about one; the outcome does not depend on this call.

The driver sent the wording exactly as written; this is a defect of the written procedure, for the same
section 11 revision.

## Departures from the written procedure

None found. `drive.py` (a scratch tool, kept here byte for byte) is the attempt 1 driver plus the amendment 3
handling; it assembled the log, and the log is committed only now, after the attempt, together with every prompt
and answer. `authoring-log.jsonl` is `state-target.json`'s `log` written one entry per line with sorted keys;
`state-target.json` is kept as the driver wrote it.

## Contents

| path | what |
|---|---|
| `authoring-inputs/target/` | every prompt sent and every answer received, byte for byte; the fact list and the E/Q split |
| `fixtures/` | the 6 target workspaces as written by the author |
| `authoring-log.jsonl` | all 121 target entries, in id order, with lint codes, group and status |
| `calls.jsonl` | one line per author call: model, command, exit code, seconds, bytes |
| `state-target.json`, `driver-*.log`, `target.done` | the driver's workspaces, split, pool check, round counts and run times |
| `drive.py` | the scratch driver that made the calls (not part of the bench) |

Nothing here is a task. Nothing under `attempt-2/` is read by `validate.py --corpus 0.2y` or by the runner (which
still reports the canonical corpus as not frozen); the canonical paths stay empty for the next attempt. A new
attempt uses new fixtures and a new author run; it does not reuse these requests.
