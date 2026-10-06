# INTERPLANE 0.2z: pre-registration for the held-out tool-discovery fix, area lead-in composed by rule

Status at registration: **pre-registered, no run exists, no task exists yet, the code for section 3 is not written.**
Nothing below describes a result. Written 2026-10-06 (UTC; dates here are UTC) after 0.2y closed unevaluated
(`PREREG-0.2y-base-set-discovery.md`, status line; `bench/heldout-0.2y/attempt-3/REPORT.md`). Changing anything here
after the first 0.2z gate run invalidates that run; every change goes in section 8 with date and reason.

## 1. What this study is, and what it inherits

0.2z tests the same fix as 0.2y: **F1, a fixed base set of workspace-read tools (`ask_user, ls, glob, grep,
read_file`) always exposed in condition B5**, with the R1 receipt fields (`PREREG-0.2y` section 2). 0.2y never
measured it: in three authoring attempts the independent author wrote 6, 3 and 3 of the 24 everyday requests that
had to name a non-file area, and amendment 4 item 16 ended 0.2y.

The base text is `docs/prereg/PREREG-0.2y-base-set-discovery.md` at aien-dev/interplane `9129936`
(sha256 `3c8a6964c4b4b79e37f6c2c6fc858faf8e53afba9d5404ce79cfa201a1a56148`) and
`bench/heldout-0.2y/AUTHORING.md` at the same commit (sha256
`2273124bf22226aa43d71ec76b6ed89499e37f2b03ebe62f0a3abb22cfdb5726`). Sections 1 to 10 of that prereg and all of that
AUTHORING.md, including amendments 1 to 4, hold for 0.2z as written, with "0.2y" read as "0.2z" and
`heldout-0.2y` read as `heldout-0.2z`, **except the changes in sections 2 and 3 below.** In particular these are
unchanged: the hypotheses (H1 B5 over B3 on discovery-needed tasks, H2 guards), the three arms A, B3, B5 and their
pins (Qwen3.5-9B, Ollama 0.34.0, Odysseus 2992bf6, sim-1 backends, `temperature 0`, `max_tokens 4096`,
`MAX_ROUNDS 8`), the 120-task composition (48 discovery-needed: 24 non-file, 24 default; 72 regression), the
author family (Codex `gpt-6-sol`) and what the author may and may not see, the leakage lint and its codes, the
24/24 group rule in id order with at most three top-ups, the 30-task calibration (A >= 40 %), the freeze order and
digests, metrics P, T, S, Safety, Validity and their thresholds, the sample size, seeds 42 (decides), 43, 44,
the failed-request rule, and the stop rule.

## 2. What is spent

Everything written for 0.2y (attempts 1 to 3: fixtures, facts, requests, authoring logs, prompts) is dev data. None
of it enters a 0.2z gate statistic or a 0.2z fixture. 0.2z uses new fixtures, a new fixture author run and a new
request author run. The 0.2x held-out set and the 44 tasks of 0.2 stay dev data, as in 0.2y.

## 3. The one design change: the area lead-in is composed by a fixed rule

**Observed (attempt 3 report, not tested further):** a brief that *allows* the request to name its everyday area did
not make this author *use* it (3 of 108 everyday requests). Each fact already carries the author's own area tag
(AUTHORING.md amendment 3). 0.2z stops relying on the author's wording for the lead-in and adds it by rule.

**C1, composition.** For every request written under brief E (the everyday brief; its text is unchanged), the
harness forms the request that is linted and, if accepted, asked:

1. If the author's text already derives exactly one non-file domain and passes the lint, it is used as written
   (`composed: false`).
2. Otherwise the lead-in for the area tag of the request's fact is put in front, from this frozen table:

   | area tag | lead-in |
   |---|---|
   | mail | `In my mail,` |
   | meetings | `From the meeting,` |
   | contacts | `In my contacts,` |
   | to-dos | `In my tasks,` |
   | notes | `In my notes,` |
   | chats | `In my chats,` |
   | models | `In my models,` |
   | memory | `In my memories,` |
   | documents | `In my documents,` |
   | images | `In the photo,` |

   joined by one space. The first letter of the author's text is lowered only when its first word is one of
   `what when which who whom whose where why how did does do is are was were can could has have had would
   should`; otherwise the text is kept byte for byte (`composed: true`).
3. The lint (unchanged) runs on the result. Acceptance into the non-file group, the default group or discard is
   decided on that text, in id order, exactly as in 0.2y.

Brief Q requests (facts tagged `other`) are never composed. A request whose fact is tagged `other` cannot reach
brief E (unchanged split by the author's own tag). The author is not told about C1, the table or the lint words.
The authoring log records, per request, the author's text, the composed text, `composed` and the lead-in used.
No person or model reads a request to choose its lead-in; the table and the fact's tag decide it.

**Override, stated explicitly.** 0.2y `AUTHORING.md` (at `9129936`, the paragraph on request numbering) says "No
request is edited, reordered, or removed by anyone." For 0.2z that sentence is replaced by: "No request is edited,
reordered or removed by anyone, except the C1 lead-in, which the harness adds by rule 2 and records next to the
author's unchanged text." Numbering, order and removal stay as written. The `heldout-0.2z/AUTHORING.md` copy (section
5) carries the replaced sentence.

**Where the area comes from.** The 0.2y authoring log has no `area` field; the tag lives in the fact file. In 0.2z
each log record also carries `area` (copied from the fact by `fact_id`), and `replay` re-derives it from the fact file
and fails on any mismatch.

**Calibration.** C1 applies the same way to the 30 calibration requests written under brief E, so the A >= 40 %
solvability check measures the text the target tasks will carry.

**Shortfall.** If a group is still short after three top-ups, 0.2z authoring ends; the shortfall is the 0.2z result
and any further attempt is a new pre-registration. This rule is the same as 0.2y amendment 4 item 16, with no
further amendment allowed on group size.

**Design check, on spent data only (2026-10-06, before any 0.2z task exists).** The C1 rule above, applied exactly
(rules 1 to 3, the table, the first-word rule) with the `lint_02y.lint_request` of `9129936` to the 108 attempt-3
target brief E requests in `bench/heldout-0.2y/attempt-3/authoring-log.jsonl`: **83 land in the non-file group**
(3 as written under rule 1, 80 composed) and 25 are discarded on the author's own words whatever the lead-in
(21 `LEAK_TOOL_TOKEN`, for example `ask` or `send`; 4 carrying `BANNED_WORD`, `DOMAIN_FILE_KEYWORD` and
`DOMAIN_MULTI` together). The lint casefolds, so the first-word rule changes no lint outcome. Each lead-in alone in
front of "what changed?" passes the lint into its intended non-file domain (email, calendar, contacts, tasks, notes,
sessions, models, memory, documents, image). Earlier choices replaced before registration: `On my to-do list,`
(`list` is a capability-name token) and `In my to-dos,` (`to-dos` is not a keyword, so it derived the default list)
failed this check; `At the meeting,`, `About the model,` and `From memory,` passed it but read as "I was there", "a
question about the assistant" and "answer without looking", so they were replaced by the table above after an
independent review, and the check was run again on the final table with the same counts. An independent review (a separate Sonnet 5.5 session, recorded in a PR #74
comment) reproduced these numbers from the same files. This check is design evidence on dev data, not a 0.2z result.

## 4. Honest limits stated now

- **Fixed phrasing.** The non-file requests carry one of ten fixed lead-ins chosen by the session that knows the
  keyword list. A 0.2z pass covers requests that name their everyday area in that form, not free phrasing of it.
  The 0.2x tasks 117 to 124 that motivated F1 were also written by the designers.
- **Selector error by construction.** The lead-in makes the selector expose the wrong (non-file) domain on all 24
  non-file tasks, which is the case F1 is meant to rescue. All three arms see the same text, so the paired B5 against
  B3 comparison is not tilted by it; A sees it too. H1 is therefore a **conditional** claim: when the selector has
  been pointed at the wrong domain, F1 rescues the task (or does not). It says nothing about how often that happens
  in natural use; no 0.2z number is a natural rate of selector error.
- **Mixed form and silent discards.** Rule 1 keeps the author's own wording when it already names one non-file area
  (3 of 83 in the design check), so the non-file group mixes composed and free requests; both are reported
  separately (descriptive). About a quarter of brief E requests (25 of 108 in the design check) are discarded on the
  author's own words, which removes requests using ordinary words such as "ask" or "send"; the 0.2y lint already did
  this, and the counts are reported.
- **Lead-in meaning.** A lead-in could read to the model as something other than "this is in that area". The
  calibration (A >= 40 % on composed text) is where a harmful reading would show; nothing beyond the lint tests it
  before calibration.
- **Group and brief are the same split.** Every non-file task comes from brief E and every default task from brief
  Q, so a difference between the two groups cannot be told apart from a difference between the two briefs. H1 is
  pooled over the 48 and does not depend on that split; the per-group figures are descriptive only.
- **Area balance and tags.** The first 24 accepted in id order are taken with no area quota (in the design check:
  meetings 9, mail 7, contacts 4, to-dos 4), and the fixture author's area tags are not checked by anyone; a wrong tag
  gives a lead-in for the wrong area. Both are reported per area, descriptive.
- **Coverage proves little,** as in 0.2y section 2: the primary metric is judged task success.
- **Areas used.** The fixture brief asks for areas from the ten-area list; attempt 3's facts used only mail,
  meetings, contacts and to-dos besides `other`. If the new fixtures do the same, the other six lead-ins are unused,
  and the result covers only the areas that occur (reported per area, descriptive).

## 5. Work required before authoring (not done at registration)

1. `bench/tools/lint_02y.py` (or a new `compose_02z` beside it): the C1 function and its tests, including every row
   of the table, the first-word rule and rule 1; the log gains `area`, the author's text, the composed text, `composed`
   and the lead-in. `replay` re-derives `area` from the fact file and the composed text from the author's text.
2. `run_bench --corpus 0.2z / 0.2z-cal` with the same completeness gate as 0.2y.
3. `bench/heldout-0.2z/AUTHORING.md`: a copy of the 0.2y file with the C1 section added; its sha256 recorded here
   in section 8 before the first author call.
4. Pins checked again before freeze (Odysseus, Ollama, model digest); any change is recorded in section 8 before
   the first model request, never after.

## 6. Machine

Runs hold the Spark quiet flag through `quietlock` (approval file for a hold over 20 minutes, as for every long run).
About 3.3 h of model time at the 0.2x median, as 0.2y section 8.

## 7. Who runs it

The INTERPLANE campaign session, after the work in section 5 is merged. Authoring, calibration and the gate run are
separate steps with their own records, as in 0.2y.

## 8. Change log

- 2026-10-06: registered.
