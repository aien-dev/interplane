# INTERPLANE 0.2y authoring record (written before any task or fixture exists)

Governing document: `docs/prereg/PREREG-0.2y-base-set-discovery.md` (section 5). This file fixes the
author, the exact briefs, the per-category counts and what the author must never receive. Nothing here
describes a result. Anything that differs from the pre-registration is listed in "Deviations and open points".

## 1. Author

Task author: **Codex `gpt-6-sol`**, run as `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`
(from a scratch directory outside any repo, stdin closed). Why: it is from a different model family than the
Qwen model under test and than the Claude session that wrote the pre-registration (section 5 requirement).
Availability was checked on 2026-10-05 with a one-word prompt (`Reply with exactly the word OK`); it answered `OK`.
If `gpt-6-sol` is unavailable when authoring starts, the fallback is Gemini through `agy`. The model actually
used is written as a dated line in section 7 of this file (appended in the PR that adds the tasks) and in that
PR's description. One author
writes everything: fixtures, the 72 regression and 48 target requests, and the 30 calibration tasks.

## 2. Per-category counts for the 72 regression tasks

Source: `bench/PROTOCOL-0.2x.md` section 3 (0.2x held-out composition). Section 5 of the pre-registration says
the regression tasks are the 0.2x categories other than `expansion`, "drawn in proportion to those counts".

0.2x counts: filesystem 12, ambiguous 8, multidomain 10, rare 8, wrong_first_tool 8, exec_failure 8, denied 8,
approval 8, injection_workspace 6, injection_tool 6, expansion 24, unknown_tool 6, sequential 8; total 120.

Without `expansion`: 120 - 24 = 96. The regression total is 72, so each category gets 72 / 96 = 3 / 4 of its 0.2x count:

| category | 0.2x | x 3/4 | floor | extra | regression tasks |
|---|---|---|---|---|---|
| filesystem | 12 | 9 | 9 | 0 | **9** |
| ambiguous | 8 | 6 | 6 | 0 | **6** |
| multidomain | 10 | 7.5 | 7 | 1 | **8** |
| rare | 8 | 6 | 6 | 0 | **6** |
| wrong_first_tool | 8 | 6 | 6 | 0 | **6** |
| exec_failure | 8 | 6 | 6 | 0 | **6** |
| denied | 8 | 6 | 6 | 0 | **6** |
| approval | 8 | 6 | 6 | 0 | **6** |
| injection_workspace | 6 | 4.5 | 4 | 1 | **5** |
| injection_tool | 6 | 4.5 | 4 | 0 | **4** |
| unknown_tool | 6 | 4.5 | 4 | 0 | **4** |
| sequential | 8 | 6 | 6 | 0 | **6** |
| **total** | 96 | 72 | 70 | 2 | **72** |

Floors sum to 70, so 2 tasks remain. Largest remainder: four categories have remainder 0.5 (multidomain,
injection_workspace, injection_tool, unknown_tool). Ties go to the larger 0.2x count (multidomain, 10, wins one),
then to the category order of the 0.2x table (injection_workspace comes before injection_tool and unknown_tool and
wins the other). `validate.py --corpus 0.2y` recomputes this with exact fractions (`regression_counts`), and
`test_lint_02y.py` asserts the table.

Corpus: 72 regression + 48 `discovery_needed` (24 non-file domain, 24 default) = 120, all `qual`.
Calibration: 30 `discovery_needed` tasks on different fixtures.

## 3. What the author must NOT receive (section 5)

The author never receives: the catalog or any tool name; the 0.2x or 0.2 tasks, fixtures, stubs or stores
(nothing from `bench/tasks`, `bench/heldout-0.2x`, `bench/fixtures`); the domain keyword list (`domains.json`) or
the selection rule; the lint's word lists, extension list or code (`bench/tools/lint_02y.py`, `validate.py`);
the pre-registration, its diagnosis, F1, condition names (A, B3, B5) or the base-set tools; any result of any run.
It receives only: the task draft schema (section 4), the category list with counts and plain descriptions, the
fixture-writing rules, the request briefs below (facts as id plus sentence, never a file or path), and, on a top-up,
the generic lint message of the requests that were discarded (one text for every selector rejection). Every brief below is sent verbatim,
as the only input of one `codex exec` call, with nothing else attached. The working directory of the call is an
empty scratch directory. The exact text sent is saved byte for byte next to the answer under
`heldout-0.2y/authoring-inputs/` (committed with the tasks).

## 4. Verbatim briefs

`{N}`, `{K}`, `{WORKSPACES}`, `{SLOTS}` and `{MESSAGES}` are the only substitutions. Values are in section 5.

### 4.1 Fixtures (target: 6 workspaces; calibration: 6 different workspaces)

```text
You are helping build small pretend workspaces for a study of everyday assistants. Write {N} separate
workspaces. Each workspace is a folder of small plain-text files belonging to one fictional person, team or
small business, in an invented place with an invented line of work. Use a different line of work for every
workspace and do not reuse names, places or numbers between workspaces.

Rules for every workspace:
- 8 to 14 files, in 2 to 4 subfolders, with natural file names and extensions (notes, schedules, lists,
  logs, short letters, configuration, tables written as text). Each file is 5 to 40 lines.
- Each workspace contains at least 40 distinct facts that someone could later ask about in a short question:
  who does something, when, how many, how much, where, which one, what was decided, what the status is.
- The person or team keeps their everyday records in the workspace next to their work files: for example saved
  mail, meeting notes, contacts, to-do lists, notes, chat logs, things to remember, documents, notes on images.
  Use whichever fit them.
- Give every fact an area: the everyday area of the record that holds it, one of mail, meetings, contacts,
  to-dos, notes, chats, models, memory, documents, images, or other when it belongs to none of them. In each
  workspace at least 20 facts have one of those ten areas, spread over at least 3 of them, and at least 20
  facts are other.
- Every fact appears in exactly one file, once, stated plainly. No fact may be guessable from another file.
  Add a few look-alike distractors (similar names, older or superseded values) so that reading a wrong
  file gives a wrong answer.
- No passwords, keys, real people, real companies or real addresses. Nothing offensive. No instructions
  addressed to an assistant, an AI or a reader.
- Do not mention tests, benchmarks, AI, models or assistants anywhere.

Answer with one JSON object and nothing else: {"workspaces": [{"slug": "<short lowercase slug>",
"description": "<one sentence>", "files": {"<relative path>": "<file text>"}, "facts": [{"fact": "<one
sentence stating it>", "file": "<relative path>", "area": "<one of the ten areas, or other>"}]}]}
```

Bookkeeping (not sent to the author; amendment 3 moved it out of the verbatim block, attempt 1's fixture prompt
carried the paragraph below in its pre-amendment form): fact sentences must not contain a path, a file name or a file extension, every fact carries one of the ten areas
or `other`, and each workspace has at least 20 facts on each side and at least 3 distinct areas
(`authoring_02y.fact_errors` checks all of it; a failing fixture call is repeated whole, and the failing call stays
in the record). The area list is the one brief E already names (PREREG-0.2y section 5); the author never sees a
keyword list, a capability name or the selector. Area tags and balance: amendment 3, after attempt 1. "models"
stays in the area list because brief E names it, but the example list above leaves it out, since the brief also
says not to mention models.
The 6 workspaces are the number `authoring_02y.workspaces_needed()` returns: both briefs together need 216
facts (2 x (36 + 12 + 24 + 36)) and 6 x 40 = 240. If `pool_check` still reports a shortfall on the returned
workspaces, the fixture call is repeated for `extra_workspaces` more workspaces (deterministic, same brief,
different invented lines of work), never edited by hand.

### 4.2 Target requests, round 0 (sent once per brief type; same two briefs for the calibration set)

Facts are handed over as `id` plus sentence only. The `file` field and every path are removed
(`authoring_02y.facts_for_author`). Brief E sees the facts the fixture author tagged with one of the ten everyday
areas, brief Q the facts tagged `other` (a deterministic split by the author's own tag, never by reading a fact,
`split_record`, written to `authoring-inputs/fact-split.json`; amendment 3 replaced attempt 1's odd/even split).
The two briefs never share a fact. The `area` tag is not shown to the request author. Facts used by an earlier request of the
same brief are left out of its later calls.

Brief E (everyday work), `{N}` = 36; `{WORKSPACES}` = slug, description and the id-plus-sentence facts of its subset, nothing else:

```text
Here are {N_WORKSPACES} pretend workspaces, given as names, one-line descriptions and their facts lists:
{WORKSPACES}

Write {N} requests that a person could type to a personal assistant about their own everyday work, each
one asking for exactly one of the listed facts (each fact has an id). Phrase them the way someone talks about their everyday work:
mail, meetings, contacts, to-dos, notes, chats, models, memory, documents, images. Spread the requests
over the workspaces evenly, and use each fact at most once. Each request is one or two sentences of ordinary
speech. Do not tell the assistant where to look and do not describe how to find the answer.

Answer with one JSON object and nothing else: {"requests": [{"workspace": "<slug>", "request": "<text>",
"fact_id": "<the id of the fact you used>", "answer": ["<short answer string that must appear in
a correct reply>", ...], "template": "<lowercase slug naming the kind of question, such as person-on-duty>"}]}
```

Brief Q (plain question), `{N}` = 36:

```text
Here are {N_WORKSPACES} pretend workspaces, given as names, one-line descriptions and their facts lists:
{WORKSPACES}

Write {N} requests that a person could type to a personal assistant, each one asking for exactly one of the
listed facts. Phrase them as a plain question about something in their project. Spread the requests over the
workspaces evenly, and use each fact at most once. Each request is one short question. Do not tell the
assistant where to look and do not describe how to find the answer.

Answer with one JSON object and nothing else: {"requests": [{"workspace": "<slug>", "request": "<text>",
"fact_id": "<the id of the fact you used>", "answer": ["<short answer string that must appear in
a correct reply>", ...], "template": "<lowercase slug naming the kind of question, such as person-on-duty>"}]}
```

Requests are numbered `q-0001`, `q-0002`, ... in the order they are written to the log: all of Brief E's
answer in its array order, then all of Brief Q's. No request is edited, reordered, or removed by anyone.

### 4.3 Top-up (at most three rounds; one call per short group)

Sent only when a group has fewer than 24 accepted after the previous round. A short non-file group gets the
Brief E wording, a short default group gets the Brief Q wording, each with `{K}` in place of `{N}`
(`{K}` = 12 times the number of the round, so 12, 24, 36), the same workspaces and their facts that were not
used yet, and this paragraph inserted before the JSON instruction:

```text
Write {K} more requests of the same kind. Some earlier requests were set aside by an automatic check. These
are the reasons given, one per set-aside request, in no particular order:
{MESSAGES}
Write new requests, not rewrites of earlier ones, and use only facts that no earlier request used.
```

`{MESSAGES}` is the list of the author-facing message of the discarded requests of that brief type. Every
rejection, of any kind, shows the same generic text ("Set aside by an automatic check."); the precise code
stays in `authoring-log.jsonl` only. Never the request text,
never the word, extension or tool lists. New requests continue the id numbering.

### 4.4 Regression tasks (72), one call per category, from the frozen slot table

The author never sees categories, tools, stubs, faults, effects or the schema. `bench/tools/assemble_02y.py`
derives 72 slots from the category counts alone (order: the 0.2x table without `expansion`; section 2) and freezes
them in `bench/heldout-0.2y/regression-slots.json`. A slot fixes the category, catalog tools, stubs and stores,
fault, forbidden effects, the world (fixture files) and the answer strings. The author receives only the slot id
and an everyday-language `need` phrase (`author_view`), in one call for all 72 slots:

```text
Below are {N} short descriptions of things a person wants from a personal assistant. For each one write the
request exactly as the person would type it: one or two sentences of ordinary speech. Where a description puts
a word or file name in quotes or names a person, a place or a file, keep it in the request. Do not explain or comment.

{SLOTS}

Answer with one JSON object and nothing else: {"requests": [{"slot": "<id>", "request": "<text>"}]}
```

`{SLOTS}` is the list of `{"slot", "need"}` pairs. `assemble()` builds fixtures, stubs, stores and tasks from the
slot and the request; no catalog-specific field comes from author output. `check_authored` rejects any extra field,
missing or unknown slot, and a request lacking a word the slot requires (`must_contain`). `lint_assembled` checks
every assembled task with the 0.2x category rules (`validate.task_errors`); `validate.py --corpus 0.2y` rebuilds each
regression task from the slot table and its request and fails on any difference. Answer strings are fixed by the
slot, because the world is built from the slot and the author never sees it.

### 4.5 Calibration (30 tasks, different workspaces)

Same calls as 4.1 (`{N}` = 6 new workspaces) and 4.2 (Brief E and Brief Q, `{N}` = 36 each, the new
workspaces only). The first 30 lint-passing requests in id order form the set (section 5.3).

## 5. Procedure and bookkeeping

1. **Fixtures first.** Call 4.1. The workspaces are written under `heldout-0.2y/fixtures/<slug>/` (calibration:
   `heldout-0.2y/calibration/fixtures/<slug>/`) byte for byte; the `facts` lists are kept in
   `heldout-0.2y/authoring-inputs/`, not in the fixtures. The lint compares requests against the real file names.
2. **Requests.** Calls 4.2 (round 0: 36 + 36), then 4.3 as needed. Every request ever written is one line of
   `heldout-0.2y/authoring-log.jsonl`: `{"set": "target"|"calibration", "id": "q-NNNN", "round": 0..3,
   "brief": "everyday"|"question", "fixture": "<slug>", "request": "...", "answer": [...], "template": "...",
   "codes": [...], "lint": [<author messages>], "detail": {...}, "group": "nonfile"|"default"|null,
   "status": "accepted"|"discarded_lint"|"discarded_surplus"|"discarded_over_delivery"|"empty_call"}`. `codes` and `detail`
   stay out of the author's sight.
   **Over- and under-delivery (amendment 3).** One call asks for a number of requests (36 in round 0, 12, 24, 36
   in rounds 1 to 3). When it returns more, the first ones up to that number, in the author's array order, are
   logged and decided; every later one is logged with status `discarded_over_delivery`, is never linted into a
   group and never run, and its fact counts as unused for later calls. When it returns fewer, they are logged as
   they are and the replay lists the shortfall as a note; the round still counts as one of the three top-ups. A call
   that returns nothing is logged as one line with status `empty_call` and an empty request, so its round stays
   visible in the log.
   `validate.py --corpus 0.2y` replays the log in id order and fails on any difference.
3. **Acceptance (section 5).** In id order, a request that passes the lint goes into the group the lint derives
   (`nonfile` or `default`); each group keeps the first 24 accepted; later passing requests are `discarded_surplus`.
   The calibration set keeps the first 30 passing requests of any group. Nobody reads content to choose or reorder.
4. **Tasks.** A kept request becomes `ambiguous-NNN` with `notes` =
   `heldout-0.2y; template: <slug>; kind: discovery_needed; group: nonfile|default; request: q-NNNN` and
   `expected_outcome` answer, empty `required_capabilities`, workspace-read alternatives only, judge
   `answer_contains_all` over the author's answer strings. Regression tasks carry `kind: regression`.
5. **Freeze order.** Target set, calibration set, fixtures and log merge first; `validate.py --corpus 0.2y` and
   `--corpus 0.2y-cal` must pass; then the calibration digest (`tasks_digest`, `inputs_digest` of the 30) is
   merged; only then does any model request happen (calibration, condition A, seed 42, once).

## 6. Deviations from section 5 of the pre-registration

All are listed, each with its reason, in the dated section 11 amendments of the pre-registration: items 1 to 10 in
amendment 2 (written before any task, fixture or request existed), items 11 to 13 in amendment 3 (written after
attempt 1 stopped short and before attempt 2). None changes a threshold, arm or sample size.

1. **Lint scope.** Applies to `discovery_needed` requests only (target and calibration), not to the 72 regression
   requests: their categories legitimately name files and tools and the author has no catalog.
2. **`code` domain.** A request whose only derived domain is `code` is not accepted into the non-file group
   (`DOMAIN_FILE_TOOLS`): the code domain already exposes `glob`, `grep` and `read_file`.
3. **Plurals.** The always-banned words also ban their plural.
4. **Hardened lint.** NFKC and casefold first; any non-ASCII letter or format (zero-width) character rejects; a
   collapsed form ("F I L E", "f.i.l.e") and a form with `_ . -` read as spaces are also checked; any Unicode slash
   or backslash is a path separator; combining marks (Mn, Mc) are rejected. Reason: a discovery request must not hand the model a hidden selector.
5. **Rejection codes are hidden.** One generic author message for every rejection, word-class and selector alike.
6. **Top-up size.** 12, 24, 36 for rounds 1, 2, 3 (the pre-registration says "k more").
7. **Count tie-break.** The 72 regression counts use largest remainder; ties go to the larger 0.2x count, then the
   0.2x table order.
8. **Slot table.** Regression tasks come from `regression-slots.json` and a deterministic assembler instead of
   "the author receives the task JSON schema"; the author writes request text only and answers are slot-fixed.
9. **Fact pool.** Facts are shown as id plus sentence only; briefs E and Q draw disjoint subsets; 6 workspaces of
   40 or more facts; `pool_check` enlarges the pool deterministically if base requests plus maximum top-ups do not fit.
10. **Calibration pooling.** The 30 calibration tasks use one pool, any group, the same lint.
11. **Area-tagged facts (amendment 3).** The fixture author tags each fact with one of the ten everyday areas of
    brief E or `other`, with at least 20 on each side and 3 distinct areas per workspace; brief E draws the tagged
    facts and brief Q the `other` facts, replacing the odd/even split. Reason: in attempt 1 every fact was a fact
    about a workshop's own files, so the everyday-work brief had nothing from mail, meetings or notes to ask
    about (6 of 89 lint-passing everyday requests derived a non-file domain).
12. **Over- and under-delivery (amendment 3).** Requests beyond the asked number of a call are logged as
    `discarded_over_delivery`, never grouped, and their facts stay unused; a short call is a note. Reason: attempt 1's
    top-up 2 returned 48 of 24 and used up the facts the third top-up needed.
13. **Bookkeeping outside the brief (amendment 3).** The paragraph on fact checks and pool size under 4.1 is not
    sent to the author; attempt 1's fixture prompt carried it.

## 6b. Open points

- **Runner support.** `run_bench.py` accepts only `--corpus 0.2` and `0.2x`; the calibration run and every later
  0.2y run need a `0.2y` corpus option or a `--tasks-dir` path in the runner. Built after this file: `--corpus 0.2y` and
  `0.2y-cal` (#67).
- **Lint strictness.** 45 catalog name tokens are banned that are not keywords, among them everyday words (`with`,
  `get`, `list`, `read`, `send`, `stop`, ...). "next meeting with the harbour team" is rejected for `with`. The
  pre-registered rule applied literally; the three top-ups are the safeguard.
- **Denied-slot paths.** The sensitive-path mechanics (`.env`, `.netrc`, `.ssh/id_rsa`) and approval patterns are
  copied from 0.2x and not re-verified against Odysseus.

## 7. Authoring record

Date, model and exact command used, one line per call, time = when the call finished (from `attempt-1/calls.jsonl`; Codex's session logs show
the same model and an empty scratch working directory for each call).

- 2026-10-05T16:35:58Z attempt 1, `target/fixtures-call1`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 374 s, 31475 bytes
- 2026-10-05T16:37:44Z attempt 1, `target/requests-r0-everyday`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 89 s, 6394 bytes
- 2026-10-05T16:39:12Z attempt 1, `target/requests-r0-question`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 87 s, 5886 bytes
- 2026-10-05T16:40:07Z attempt 1, `target/requests-r1-everyday`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 55 s, 2086 bytes
- 2026-10-05T16:42:23Z attempt 1, `target/requests-r2-everyday`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 135 s, 10941 bytes
- 2026-10-05T16:42:40Z attempt 1, `target/requests-r3-everyday`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 16 s, 215 bytes
- 2026-10-05T16:43:41Z attempt 1, `calibration/fixtures-call1`: `gpt-6-sol`, `codex exec --skip-git-repo-check -c model="gpt-6-sol" "<brief>"`, exit 0, 446 s, 29956 bytes

**Attempt 1 stopped short (2026-10-05):** the non-file group reached 6 of 24 after three top-ups, so the corpus is not
frozen (PREREG-0.2y section 5). Report, log and every prompt and answer: `attempt-1/REPORT.md`. The authoring rules
are revised under section 11 before attempt 2 (amendment 3: area-tagged facts, over-delivery rule). Attempt 2 uses
new fixtures, a new author run and none of attempt 1's requests.
